//! Check file identity before any output is created or overwritten.
use std::os::unix::fs::symlink;
use std::{fs, path::Path, process::Command};

fn rejected(bin: &str, input: &Path, text: &Path, json: &Path) {
    let result = Command::new(bin)
        .arg(input)
        .arg("--text")
        .arg(text)
        .arg("--json")
        .arg(json)
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("same file"));
}

fn main() {
    let bin = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "./termshot".into());
    let dir = std::env::temp_dir().join(format!(
        "termshot-aliases-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let input = dir.join("input.pty");
    fs::write(&input, "hello").unwrap();
    let target = dir.join("target.txt");
    let alias = dir.join("alias.txt");
    symlink("target.txt", &alias).unwrap();
    for (a, b) in [(&alias, &target), (&target, &alias)] {
        rejected(&bin, &input, a, b);
        assert!(!target.exists(), "preflight must not create the target");
        assert_eq!(fs::read_link(&alias).unwrap(), Path::new("target.txt"));
    }
    // Chains, absolute targets, directory symlinks and parent components.
    let chain = dir.join("chain.txt");
    symlink(&alias, &chain).unwrap();
    fs::create_dir(dir.join("sub")).unwrap();
    let directory_alias = dir.join("dir-link");
    symlink("sub", &directory_alias).unwrap();
    rejected(&bin, &input, &chain, &directory_alias.join("../target.txt"));
    assert!(!target.exists());
    // Existing hard links alias both other outputs and inputs.
    fs::write(&target, "KEEP").unwrap();
    let hard = dir.join("hard.txt");
    fs::hard_link(&target, &hard).unwrap();
    rejected(&bin, &input, &target, &hard);
    rejected(&bin, &input, &hard, &target);
    assert_eq!(fs::read(&target).unwrap(), b"KEEP");
    let input_alias = dir.join("input-alias.pty");
    fs::hard_link(&input, &input_alias).unwrap();
    let result = Command::new(&bin)
        .arg(&input)
        .arg("--text")
        .arg(&input_alias)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("destroy the input"));
    assert_eq!(fs::read(&input).unwrap(), b"hello");
    let font = dir.join("font.ttf");
    let font_alias = dir.join("font-alias.ttf");
    fs::write(&font, "FONT CONTENT").unwrap();
    fs::hard_link(&font, &font_alias).unwrap();
    let result = Command::new(&bin)
        .arg(&input)
        .arg("--font")
        .arg(&font)
        .arg("--text")
        .arg(&font_alias)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(2));
    assert_eq!(fs::read(&font).unwrap(), b"FONT CONTENT");
    // A cycle must terminate with the normal I/O error and clean up any
    // unrelated output created earlier in preflight, without removing the link.
    let cycle = dir.join("cycle");
    symlink("cycle", &cycle).unwrap();
    let spare = dir.join("spare.txt");
    let result = Command::new(&bin)
        .arg(&input)
        .arg("--text")
        .arg(&spare)
        .arg("--json")
        .arg(&cycle)
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    assert!(!spare.exists());
    assert_eq!(fs::read_link(&cycle).unwrap(), Path::new("cycle"));
    // Distinct files with identical contents must remain valid destinations.
    let distinct = dir.join("distinct.txt");
    fs::write(&distinct, "KEEP").unwrap();
    assert!(Command::new(&bin)
        .arg(&input)
        .arg("--text")
        .arg(&target)
        .arg("--json")
        .arg(&distinct)
        .status()
        .unwrap()
        .success());
    assert!(fs::read_to_string(&target).unwrap().starts_with("hello\n"));
    assert!(fs::read_to_string(&distinct).unwrap().starts_with('{'));
    println!("ok, dangling/chained/directory symlinks and hard-link input/output aliases; contents preserved");
}
