//! 自动发现可能作为图库的目录。
//!
//! 用户的原话是「自动推荐扫描所有有图片的目录，我可以选择勾选哪部分目录作为图库」。
//! 所以这里只做**发现**：走一遍、数一数、把候选连同张数交出去。
//! 采纳哪些由用户在设置页勾，不在这里替他决定。

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::LumenError;

/// 遍历条目数的上限。
///
/// `iCloud Photos` 这类同步目录走一遍要几分钟，而且可能挂着几十万个文件。
/// 撞上上限就带着已有结果返回，并在 `truncated` 里说明——悄悄返回半个结果
/// 会让人以为「有照片的目录就这些」。
const MAX_ENTRIES: u64 = 200_000;

/// 最多回多少个候选。够覆盖任何正常机器，同时不让 IPC 负载失控。
const MAX_CANDIDATES: usize = 300;

/// 一个「这里可能是个图库」的候选目录。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryCandidate {
    pub path: String,
    /// 目录名。根目录（`D:\`）没有名字，回退到整条路径。
    pub name: String,
    /// 这个目录**自己**直接含多少张照片，不算子目录。
    pub photos: u64,
    /// 连同子目录一共多少张。
    pub photos_deep: u64,
    /// 已经在图库目录列表里。
    pub already_added: bool,
}

/// 一次发现的完整结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryReport {
    /// 从哪个目录开始找的。
    pub base: String,
    /// 按「一共多少张」从多到少排好的候选。
    pub candidates: Vec<LibraryCandidate>,
    /// 撞到遍历上限了，结果不完整。
    pub truncated: bool,
    /// 实际看了多少个目录条目。
    pub visited: u64,
}

/// 走一遍 `base` 下面最多 `max_depth` 层，找出所有含照片的目录。
///
/// # 两个计数各回答一个问题
///
/// `photos`（自己直接含几张）回答「这个目录本身是不是一个照片目录」，
/// `photos_deep`（连子目录一共几张）回答「选它一共能拿到多少」。
/// 只给一个数的话，用户没法判断该选 `D:\Pictures` 还是它下面那二十个日期目录。
pub fn discover(
    base: &Path,
    max_depth: u32,
    existing: &[PathBuf],
) -> Result<DiscoveryReport, LumenError> {
    if !base.is_dir() {
        return Err(LumenError::io(format!(
            "{} 不是一个存在的目录",
            base.display()
        )));
    }

    let mut direct: HashMap<PathBuf, u64> = HashMap::new();
    let mut deep: HashMap<PathBuf, u64> = HashMap::new();
    let mut visited: u64 = 0;
    let mut truncated = false;

    // 广度优先：浅层先看完。撞上限时至少顶层是齐的；深度优先会把预算全耗在
    // 一棵巨大的子树上，返回的候选却只有那一棵，看起来像是别处没照片。
    let mut queue: VecDeque<(PathBuf, u32)> = VecDeque::new();
    queue.push_back((base.to_path_buf(), 0));

    'walk: while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_ENTRIES {
                truncated = true;
                break 'walk;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();

            if kind.is_dir() {
                // 和扫描器用同一份跳过名单（`lumenfold_index::is_skipped`），
                // 否则这里会把 `.git` 推荐成图库，而扫描进去又什么都找不到。
                if depth >= max_depth || lumenfold_index::is_skipped(&entry.file_name()) {
                    continue;
                }
                queue.push_back((path, depth + 1));
                continue;
            }
            if !kind.is_file() {
                continue;
            }

            let is_photo = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(lumenfold_index::is_photo_extension);
            if !is_photo {
                continue;
            }

            *direct.entry(dir.clone()).or_insert(0) += 1;
            // 沿祖先链一直加到 base 为止。`starts_with` 把高过 base 的祖先挡在外面，
            // 否则用户选 `D:\Pictures` 会把 `D:\` 也算成候选。
            for ancestor in dir.ancestors() {
                if !ancestor.starts_with(base) {
                    break;
                }
                *deep.entry(ancestor.to_path_buf()).or_insert(0) += 1;
            }
        }
    }

    let mut candidates: Vec<LibraryCandidate> = deep
        .iter()
        .filter(|(_, count)| **count > 0)
        .map(|(path, count)| LibraryCandidate {
            path: path.to_string_lossy().into_owned(),
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
            photos: direct.get(path).copied().unwrap_or(0),
            photos_deep: *count,
            already_added: existing.iter().any(|root| root == path),
        })
        .collect();

    // 照片多的在前：用户多半就是冲着那个「一共一万两千张」的目录来的。
    candidates.sort_by(|a, b| {
        b.photos_deep
            .cmp(&a.photos_deep)
            .then_with(|| a.path.cmp(&b.path))
    });
    if candidates.len() > MAX_CANDIDATES {
        candidates.truncate(MAX_CANDIDATES);
        truncated = true;
    }

    Ok(DiscoveryReport {
        base: base.to_string_lossy().into_owned(),
        candidates,
        truncated,
        visited,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一个用完就删的临时目录。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "lumenfold-discover-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn touch_photo(dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).expect("create dir");
        std::fs::write(dir.join(name), b"not really a photo").expect("write file");
    }

    /// 两个计数要各说各的：一个目录可以直接没有照片、但子树里有很多。
    ///
    /// 这正是自动推荐要回答的问题——手机相册的根目录往往一张照片都不直接放，
    /// 全在它下面的日期目录里。
    #[test]
    fn counts_direct_and_recursive_photos_separately() {
        let dir = TempDir::new("counts");
        let root = &dir.0;
        let nested = root.join("2024").join("05");
        touch_photo(&nested, "a.jpg");
        touch_photo(&nested, "b.png");
        touch_photo(&root.join("2024"), "c.jpeg");
        // 非照片不该被数进去。
        std::fs::write(root.join("notes.txt"), b"x").expect("write text");

        let report = discover(root, 4, &[]).expect("discover");
        let by_path = |suffix: &str| {
            report
                .candidates
                .iter()
                .find(|candidate| candidate.path.ends_with(suffix))
                .unwrap_or_else(|| panic!("no candidate ending in {suffix}"))
        };

        let leaf = by_path("05");
        assert_eq!(leaf.photos, 2, "叶子目录自己直接有两张");
        assert_eq!(leaf.photos_deep, 2, "它下面没有更深的东西");

        let year = by_path("2024");
        assert_eq!(year.photos, 1, "2024 自己直接有一张");
        assert_eq!(year.photos_deep, 3, "连同 05 一共三张");

        // 根目录自己不直接放照片，但子树里有三张——点「添加」能拿到的就是它。
        let top = report
            .candidates
            .iter()
            .find(|candidate| Path::new(&candidate.path) == root.as_path())
            .expect("the base itself is a candidate");
        assert_eq!(top.photos, 0, "根目录自己不直接放照片");
        assert_eq!(top.photos_deep, 3);
    }

    /// 深度是硬上限：超出去的照片一张都不该被推荐。
    #[test]
    fn max_depth_is_respected() {
        let dir = TempDir::new("depth");
        let root = &dir.0;
        touch_photo(&root.join("a").join("b"), "deep.jpg");

        let shallow = discover(root, 1, &[]).expect("discover");
        assert!(
            shallow.candidates.is_empty() || shallow.candidates.iter().all(|c| c.photos_deep == 0),
            "深度 1 看不到 a/b 里面的照片"
        );

        let deep = discover(root, 3, &[]).expect("discover");
        assert!(
            deep.candidates.iter().any(|c| c.path.ends_with("b")),
            "深度够时应该找到它"
        );
    }

    /// 已经在图库里的目录要被标出来，界面才能把它显示成「已添加」而不是可勾选项。
    #[test]
    fn already_added_is_flagged_not_hidden() {
        let dir = TempDir::new("added");
        let root = &dir.0;
        touch_photo(root, "a.jpg");

        let report = discover(root, 2, &[root.to_path_buf()]).expect("discover");
        let candidate = report
            .candidates
            .iter()
            .find(|candidate| candidate.path == root.to_string_lossy())
            .expect("the base itself is a candidate");
        assert!(candidate.already_added);
    }

    /// 目录不存在时要明确报错，而不是回一个空列表让用户以为「这里没有照片」。
    #[test]
    fn a_missing_base_is_an_error_not_an_empty_list() {
        let dir = TempDir::new("missing");
        let absent = dir.0.join("does-not-exist");
        assert!(discover(&absent, 2, &[]).is_err());
    }
}
