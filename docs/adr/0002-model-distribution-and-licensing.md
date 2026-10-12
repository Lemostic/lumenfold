# ADR-0002：模型分发与许可

- **状态**：已接受
- **日期**：2026-10-07
- **决策者**：项目发起人

---

## 背景

ADR-0001 §6 已经定了「模型权重不进安装包也不进仓库，改为后台可续传下载 + 本地模型注册表」，但那条决策只写了结论，没有把依据记下来。本次决策要把依据、边界和核查结果固化成可复核的记录。

模型权重的许可状况有一个反复出现的坑：**代码的许可不等于权重的许可**。以 InsightFace 为例，其 Python 库的代码是 MIT，但官方 README 明确写着随库提供的预训练模型仅限非商业研究用途，自动下载和手动下载都适用；而 `detection/scrfd/LICENSE` 里那份 Apache-2.0 是 Open-MMLab 的 SCRFD **代码**许可，覆盖的是检测网络的实现，不是打包好的权重。把代码许可当成权重许可是这类项目最常见的合规事故来源。

分发方式决定了责任归属。一旦权重进了安装包，项目就构成了分发方，需要为每一位拿到安装包的人是否合规负责；权重如果只出现在用户的机器上，那么做出下载动作、接受许可的是用户自己。这个差别不是文书细节，它决定了开源项目能不能既用非商用权重做能力验证，又不把自己的许可证搭进去。

## 决策

### 1. 总则：权重永不进入本项目的分发渠道

模型权重**不进安装包、不进仓库、不由本项目以任何形式再分发**，包括但不限于：发布包的 resources 目录、Git LFS 指针指向的对象、GitHub Release 附件、容器镜像层、以及官方文档里给出的直接下载链接。

替代方案是**本地模型注册表**：宿主只保存模型的元数据（id、名称、任务类型、许可标识、来源 URL、SHA-256、体积），权重本体由用户在安装后自行获取。

**理由**：下载便利性可以由下载器提供，红包成本不能由项目承担。把下载动作留在用户机器上，责任归属就清楚了。

### 2. 许可核查结果

以下是本次实际查阅来源得到的结论。每条注明所读来源与是否允许商业使用。

| 模型 | 许可 | 是否允许商用 | 来源 |
|---|---|---|---|
| InsightFace `buffalo_l` 模型包（SCRFD + ArcFace 等） | 非商业研究用途，自动下载与手动下载同样适用 | 否 | 官方库说明「The pretrained models provided with this library are for **non-commercial research only**, whether downloaded automatically or manually」：<https://pypi.org/project/insightface/0.7>；商业授权需单独洽谈：<https://www.insightface.ai/solutions/face-recognition-licensing> |
| InsightFace SCRFD **代码** | Apache-2.0（Copyright 2018-2019 Open-MMLab） | 是（仅限代码） | <https://cdn.jsdelivr.net/gh/deepinsight/insightface@master/detection/scrfd/LICENSE> |
| OpenCV YuNet 人脸检测权重 | MIT（Copyright (c) 2020 Shiqi Yu） | 是 | 模型目录自带 LICENSE：<https://hf-mirror.com/opencv/opencv_zoo/resolve/main/models/face_detection_yunet/LICENSE> |
| AdaFace 代码 | MIT（Copyright (c) 2022 Minchul Kim） | 是（仅限代码） | <https://github.com/mk-minchul/AdaFace/blob/master/LICENSE> |
| AdaFace 官方预训练权重 | **待核查**。权重许可与训练数据集许可绑定，模型卡要求「follow the license of the training dataset」，而各权重所依据的数据集（WebFace4M / MS1MV2 / CASIA-WebFace）条款各不相同；社区亦就 MIT 是否覆盖官方 checkpoint 存在公开询问未获明确答复 | 待核查 | 权重模型卡：<https://hf-mirror.com/minchul/cvlface_adaface_ir101_webface4m/raw/main/README.md>；许可归属询问：<https://github.com/mk-minchul/AdaFace/issues/175> |
| OpenAI CLIP 图像编码器**代码** | MIT（Copyright (c) 2021 OpenAI） | 是（仅限代码） | <https://cdn.jsdelivr.net/gh/openai/CLIP@main/LICENSE> |
| OpenAI CLIP **权重**（`clip-vit-base-patch32`） | MIT 代码许可覆盖权重，但模型卡另行声明「**Any** deployed use case of the model, whether commercial or not, is currently out of scope」，且仅在英语语境下评估过 | 有条件：商用允许，但部署用途需自行承担领域适配责任 | 模型卡：<https://hf-mirror.com/openai/clip-vit-base-patch32/raw/main/README.md> |
| sqlite-vec | MIT（Copyright (c) 2024 Alex Garcia）或 Apache-2.0 双许可 | 是 | <https://cdn.jsdelivr.net/gh/asg017/sqlite-vec@main/LICENSE-MIT>；<https://cdn.jsdelivr.net/gh/asg017/sqlite-vec@main/LICENSE-APACHE> |

**待核查项的处置**：AdaFace 官方权重在依据明确之前不进入默认注册表，只作为用户自行提供的模型存在。宿主不代为断言其许可，也不为其提供下载入口。

### 3. 默认注册表与用户自备的划分

**默认注册表（宿主提供下载入口）**

- YuNet 人脸检测权重（MIT）。这是人脸能力默认链路上唯一一个许可干净的环节，因此检测先行、识别可选。
- CLIP 图像编码器（MIT 代码 + 模型卡部署限制声明）。语义检索的默认编码器。

**用户自备（宿主不提供下载入口，不代为断言许可）**

- InsightFace `buffalo_l`（含 ArcFace 系权重）。用户若持有商业授权或接受非商用条款，自行放置到模型目录，宿主识别并加载，但在界面上按非商用处理。宿主可以提供路径配置与加载能力，不提供获取能力。
- AdaFace 及一切预训练权重。原因见 §2 待核查项。
- 用户自行训练或从其他来源获得的任意 ONNX 权重。

**划分原则**：分界线是**宿主能否替用户做出许可判断**。能（MIT / Apache-2.0，且无额外部署限制）的进默认注册表；不能的进用户自备区。

### 4. 下载器

安装完成后，用户通过应用内的模型面板按需下载。下载器满足以下要求：

- **许可先于首字节**。点击下载先弹出该模型的许可全文与商用标识，用户显式接受后才发出第一个 HTTP 请求。接受动作留痕。
- **可续传**。走 HTTP Range 断点续传。中断后重启从已下载偏移继续，不重头再来。
- **完整性校验**。下载完成后校验 SHA-256，只有校验通过才把模型标记为 ready。校验失败视为损坏，删除分片并回到 absent 状态，不允许「先凑合用」。
- **后台运行，有界并发**。下载在后台执行，并发数有上限，不与其他后台任务争抢。
- **不阻塞 UI 线程**。进度通过事件推送到前端，界面在任何时刻保持可交互。
- **受同一缓存预算约束**。模型缓存与缩略图缓存共用同一套内容寻址 + LRU 预算机制（见 ROADMAP §4「缓存膨胀」行），避免模型堆积绕过用户设置的磁盘上限。

### 5. 非商用标识的界面义务

注册表中任一模型被标记为非商用时，依赖该模型的功能必须显示明确的「非商用」提示，而不是只在设置页里放一行小字。宿主不因为自己不分发权重，就认为自己对使用者的行为没有提示义务。

### 6. 宿主许可不传导

本项目宿主与 SDK 采用 AGPL-3.0-or-later（ADR-0001 §5）。AGPL 的效力范围是本项目的源码，不覆盖第三方模型权重，也**不能把非商用权重变成可商用权重**。许可不可向上传导，也不可向下传导：权重不因宿主是 AGPL 而获得 AGPL，宿主也不因某个权重非商用而获得任何许可。

## 影响

**正面**

- 非商用权重（如 InsightFace 系）可以在不触碰项目自身许可的前提下被用于能力验证与个人使用。
- 默认链路上出现的每一个模型，许可都已经逐条核实过，不存在「大概是 MIT」的模糊地带。
- 注册表是唯一通道，许可只作为条目上的一个字段存在。将来出现许可清晰的商用权重，与现有的非商用权重并列成条目，不需要为它新建一套机制，注册表的扩展性不被某一家模型厂商的条款卡住。
- 用户的磁盘上限对模型和缩略图一体生效，不会出现模型绕过预算堆积的情况。

**负面**

- 首次使用人脸或语义检索时多一步下载与许可确认，转化率会低于开箱即用。
- 注册表需要维护：每个模型的来源 URL、SHA-256 与许可标识都可能随上游变更而失效，需要有人盯。
- AdaFace 权重在依据明确前无法进默认注册表，人脸识别链路短期内只能依赖用户自备权重，或等待许可澄清。
- CLIP 的模型卡把部署用途划为 out of scope，这件事必须如实呈现给用户，不能因为代码是 MIT 就略过不提。

## 后续**

- **权重许可复核机制**：模型注册表的每一项都需要记录核查日期与来源；上游若变更许可，需要有发现与更新路径，而不是让陈旧的 SHA-256 与许可标识长期留在发布版里。
- **AdaFace 权重许可跟踪**：待官方对权重与数据集许可归属给出明确说明后再评估是否纳入默认注册表。
- **致谢与第三方声明**：用户下载模型时需能看到完整的第三方组件声明，与 ADR-0001 §5 的发行包声明要求保持一致。
- **编号说明**：本 ADR 使用 0002。ADR-0001 §后续 中「ADR-0002：数据库 schema v1 与迁移框架细则」一条与之冲突，该条目应改为 ADR-0003 或另择编号，此处按本 ADR 为 0002 处理。