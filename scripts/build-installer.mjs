#!/usr/bin/env node
/**
 * 出安装包。
 *
 * `pnpm tauri build` 本身只是 cargo build 加一步打包，产物却散落在
 * target/release/bundle 下面有好几个文件，路径要靠人记。这次把它固化成一条命令：
 * 先查前置条件，再构建，最后只认「构建开始之后新产出」的安装包并打印路径与体积。
 *
 * 按时间筛而不是直接列目录，是为了不把上一轮遗留的旧包当成这次的成果报出来。
 * 没筛到新包就当失败处理：静默交付一个陈旧安装包，比报错更糟。
 *
 * 不引入任何依赖。Node 是这个仓库已经要求的前置条件，
 * 跨 Windows / macOS / Linux 都能直接跑。
 */

import { spawn, spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const TAURI_DIR = join(ROOT, "apps", "desktop", "src-tauri");
const CONF_PATH = join(TAURI_DIR, "tauri.conf.json");

/** 各平台可能产出的安装包后缀。够用即可，不追求覆盖 Tauri 的全部 target。 */
const INSTALLER_EXT = new Set([
  ".msi",
  ".exe",
  ".dmg",
  ".deb",
  ".rpm",
  ".appimage",
  ".gz",
  ".app",
]);

/** 主程序本体，不算安装包，但体积最能说明前端资源嵌进去没有。 */
const BINARY_NAME = "lumenfold-desktop.exe";

function fail(message) {
  console.error(`\n✗ ${message}\n`);
  process.exit(1);
}

function step(label) {
  console.log(`\n▸ ${label}`);
}

/* ---------------------------------------------------------------- 前置检查 */

/**
 * 图标缺失要等到 tauri 打包阶段才报错，那时已经编译了四分钟。
 * 放在最前面查掉，省掉整轮等待。
 */
function checkIcons(conf) {
  const icons = conf.bundle?.icon ?? [];
  if (icons.length === 0) return;

  const missing = icons.filter((icon) => !existsSync(join(TAURI_DIR, icon)));
  if (missing.length > 0) {
    fail(`tauri.conf.json 声明的图标不存在：\n  ${missing.join("\n  ")}\n请先补齐 icons/ 下的文件。`);
  }
}

function checkToolchain() {
  const major = Number.parseInt(process.versions.node.split(".")[0] ?? "0", 10);
  if (major < 20) {
    fail(`需要 Node 20 或更高，当前是 ${process.versions.node}。`);
  }

  // Rust 侧同理：没有 cargo 就别浪费时间先跑前端构建。
  const cargo = spawnSync("cargo", ["--version"], { stdio: "ignore", shell: false });
  if (cargo.error || cargo.status !== 0) {
    fail(
      "PATH 里找不到 cargo。Windows 上安装 Rust 后需要新开一个终端，\n让 rustup 写入的 PATH 生效。",
    );
  }
}

/* ------------------------------------------------------------------ 构建 */

/**
 * 决定怎么调起 pnpm。
 *
 * Windows 上 pnpm 是 pnpm.cmd，child_process 不带 shell 直接 spawn .cmd 会抛 EINVAL，
 * 所以这里优先走 npm_execpath：pnpm 运行脚本时会把它自己的 JS 入口写进这个环境变量，
 * 用 node 直接执行它就绕开了 .cmd，参数也不必拼进命令行字符串。
 * 直接用 node 调脚本、拿不到 npm_execpath 时才退回 shell（此时参数是写死的，没有注入面）。
 */
function pnpmInvocation() {
  const execPath = process.env.npm_execpath;
  if (execPath && /\.(c|m)?js$/i.test(execPath) && existsSync(execPath)) {
    return { command: process.execPath, args: [execPath, "tauri:build"], shell: false };
  }
  return {
    command: "pnpm",
    args: ["tauri:build"],
    shell: process.platform === "win32",
  };
}

function runBuild() {
  const { command, args, shell } = pnpmInvocation();

  return new Promise((resolvePromise, rejectPromise) => {
    const child = spawn(command, args, {
      cwd: ROOT,
      stdio: "inherit",
      shell,
    });

    child.on("error", (error) => {
      rejectPromise(new Error(`起不来 ${command}：${error.message}`));
    });
    child.on("close", (code) => {
      if (code === 0) resolvePromise();
      else rejectPromise(new Error(`pnpm tauri:build 退出码 ${code}`));
    });
  });
}

/* ------------------------------------------------------------------ 产物 */

/** 递归收集 bundle 目录下 mtime 不早于 since 的安装包。 */
function collectInstallers(since) {
  const bundleRoot = join(TAURI_DIR, "target", "release", "bundle");
  if (!existsSync(bundleRoot)) return [];

  const found = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(full);
        continue;
      }
      const name = entry.name.toLowerCase();
      const isInstaller = [...INSTALLER_EXT].some((ext) => name.endsWith(ext));
      if (!isInstaller) continue;
      if (statSync(full).mtimeMs < since) continue;
      found.push(full);
    }
  };

  walk(bundleRoot);
  return found.sort();
}

function formatSize(bytes) {
  return `${(bytes / 1024 / 1024).toFixed(2)} MiB`;
}

/* ------------------------------------------------------------------ 主流程 */

async function main() {
  if (!existsSync(CONF_PATH)) {
    fail(`找不到 ${relative(ROOT, CONF_PATH)}，请在仓库根目录运行。`);
  }

  step("检查前置条件");
  const conf = JSON.parse(readFileSync(CONF_PATH, "utf8"));
  checkToolchain();
  checkIcons(conf);
  console.log(`  Node ${process.versions.node}，cargo 就绪，图标齐备。`);

  // 记下起始时间，用来判断哪些产物是这一轮新生成的。
  const startedAt = Date.now();
  const binaryPath = join(TAURI_DIR, "target", "release", BINARY_NAME);
  const binaryBefore = existsSync(binaryPath) ? statSync(binaryPath).mtimeMs : 0;

  step(`构建安装包（${conf.productName ?? "app"} ${conf.version ?? "?"}）`);
  console.log("  Rust release 编译通常要几分钟，请耐心等待。\n");

  try {
    await runBuild();
  } catch (error) {
    fail(error.message);
  }

  step("核对产物");
  const installers = collectInstallers(startedAt);

  if (installers.length === 0) {
    fail(
      "构建结束了，但没找到这一轮新产出的安装包。\n" +
        "可能是 bundle.targets 被改成了空，或者构建实际失败了。",
    );
  }

  for (const file of installers) {
    console.log(`  ${relative(ROOT, file)}  ${formatSize(statSync(file).size)}`);
  }

  if (existsSync(binaryPath)) {
    const mtime = statSync(binaryPath).mtimeMs;
    const fresh = mtime >= binaryBefore;
    console.log(
      `\n  主程序 ${BINARY_NAME} ${formatSize(statSync(binaryPath).size)}` +
        `${fresh ? "" : "（未重新链接，可能是增量构建复用了旧产物）"}`,
    );
  }

  console.log(`\n✓ 打包完成，${installers.length} 个安装包。`);
  console.log("  签名与公证尚未配置，只能安装到本机自测。\n");
}

await main();
