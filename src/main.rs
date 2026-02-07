use clap::Parser;
use git2::Repository;
use rayon::prelude::*;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const TARGETS: &[&str] = &[
    // Java / Gradle / Kotlin
    "build",
    ".gradle",
    // .NET / generic
    "bin",
    "obj",
    // Node
    "node_modules",
    // Rust
    "target",
    // Python
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".tox",
];

#[derive(Parser)]
#[command(about = "Purge gitignored build artifacts and caches")]
struct Args {
    /// Directory to scan
    path: PathBuf,

    /// Max depth to search for repos
    #[arg(short = 'L', default_value = "3")]
    depth: usize,

    /// Show what would be deleted without deleting
    #[arg(short = 'n', long)]
    dry_run: bool,

    /// Skip confirmation prompt
    #[arg(short, long)]
    yes: bool,
}

struct Purge {
    path: PathBuf,
    size: u64,
}

fn find_repos(base: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    collect_repos(base, max_depth, 0, &mut repos);
    repos.sort();
    repos
}

fn collect_repos(dir: &Path, max_depth: usize, depth: usize, repos: &mut Vec<PathBuf>) {
    if depth > max_depth {
        return;
    }
    if dir.join(".git").exists() {
        repos.push(dir.to_path_buf());
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && !path.is_symlink() {
            collect_repos(&path, max_depth, depth + 1, repos);
        }
    }
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() && !ft.is_symlink() {
                stack.push(entry.path());
            } else if ft.is_file() {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

fn find_purgeable(repo_path: &Path) -> Vec<Purge> {
    let Ok(repo) = Repository::open(repo_path) else {
        return Vec::new();
    };

    let mut results = Vec::new();
    scan_dir(&repo, repo_path, repo_path, &mut results);
    results
}

fn scan_dir(repo: &Repository, repo_root: &Path, dir: &Path, out: &mut Vec<Purge>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || path.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_str().unwrap_or("");

        if TARGETS.contains(&name) {
            let rel = path.strip_prefix(repo_root).unwrap_or(&path);
            let check = format!("{}/", rel.display());
            if repo.is_path_ignored(Path::new(&check)).unwrap_or(false) {
                out.push(Purge {
                    path: path.clone(),
                    size: dir_size(&path),
                });
            }
            continue;
        }

        // Skip .git and other hidden dirs
        if name.starts_with('.') {
            continue;
        }
        scan_dir(repo, repo_root, &path, out);
    }
}

fn human_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.0} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn run(args: Args) -> Result<(), String> {
    let base = args
        .path
        .canonicalize()
        .map_err(|_| format!("cactus: cannot access '{}'", args.path.display()))?;

    let repos = find_repos(&base, args.depth);
    if repos.is_empty() {
        return Err(format!("No git repos found in {}", base.display()));
    }

    let all_purges: Vec<(PathBuf, Vec<Purge>)> = repos
        .par_iter()
        .map(|r| (r.clone(), find_purgeable(r)))
        .filter(|(_, p)| !p.is_empty())
        .collect();

    if all_purges.is_empty() {
        println!("Nothing to purge.");
        return Ok(());
    }

    let mut total_size = 0u64;
    let mut total_count = 0usize;

    for (repo, purges) in &all_purges {
        let rel = repo.strip_prefix(&base).unwrap_or(repo);
        println!("\x1b[1m{}\x1b[0m", rel.display());
        for p in purges {
            let dir_rel = p.path.strip_prefix(repo).unwrap_or(&p.path);
            println!("  \x1b[31m{}\x1b[0m  {}", dir_rel.display(), human_size(p.size));
            total_size += p.size;
            total_count += 1;
        }
    }

    println!(
        "\n{total_count} dirs, {} reclaimable",
        human_size(total_size)
    );

    if args.dry_run {
        return Ok(());
    }

    if !args.yes {
        print!("Purge? [y/N] ");
        io::stdout().flush().ok();
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .map_err(|e| format!("Failed to read input: {e}"))?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("Aborted.");
            return Ok(());
        }
    }

    let mut freed = 0u64;
    let mut errors = 0usize;
    for (_, purges) in &all_purges {
        for p in purges {
            match fs::remove_dir_all(&p.path) {
                Ok(()) => freed += p.size,
                Err(e) => {
                    eprintln!("cactus: {}: {e}", p.path.display());
                    errors += 1;
                }
            }
        }
    }

    println!("Freed {}", human_size(freed));
    if errors > 0 {
        return Err(format!("{errors} dirs failed to remove"));
    }
    Ok(())
}

fn main() {
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    if let Err(e) = run(Args::parse()) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn init_repo(tmp: &Path, name: &str, gitignore: &str) -> PathBuf {
        let dir = tmp.join(name);
        fs::create_dir_all(&dir).unwrap();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init", "-q"])
            .current_dir(&dir)
            .status()
            .unwrap();
        if !gitignore.is_empty() {
            fs::write(dir.join(".gitignore"), gitignore).unwrap();
        }
        dir
    }

    #[test]
    fn purges_gitignored_build_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = init_repo(tmp.path(), "proj", "build/\n");
        fs::create_dir_all(repo.join("build")).unwrap();
        fs::write(repo.join("build/out.jar"), "fake").unwrap();

        let purges = find_purgeable(&repo);
        assert_eq!(purges.len(), 1);
        assert!(purges[0].path.ends_with("build"));
    }

    #[test]
    fn skips_tracked_build_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // No gitignore — build/ is NOT ignored
        let repo = init_repo(tmp.path(), "proj", "");
        fs::create_dir_all(repo.join("build")).unwrap();
        fs::write(repo.join("build/out.jar"), "fake").unwrap();

        let purges = find_purgeable(&repo);
        assert!(purges.is_empty());
    }

    #[test]
    fn finds_nested_node_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = init_repo(tmp.path(), "mono", "node_modules/\n");
        fs::create_dir_all(repo.join("packages/web/node_modules")).unwrap();
        fs::write(
            repo.join("packages/web/node_modules/fake.js"),
            "module.exports = {}",
        )
        .unwrap();

        let purges = find_purgeable(&repo);
        assert_eq!(purges.len(), 1);
        assert!(purges[0].path.ends_with("node_modules"));
    }

    #[test]
    fn finds_multiple_targets_in_one_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = init_repo(tmp.path(), "full", "build/\nnode_modules/\ntarget/\n");
        fs::create_dir_all(repo.join("build")).unwrap();
        fs::create_dir_all(repo.join("node_modules")).unwrap();
        fs::create_dir_all(repo.join("target")).unwrap();

        let purges = find_purgeable(&repo);
        assert_eq!(purges.len(), 3);
    }

    #[test]
    fn dir_size_computes_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("test");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("a.txt"), "hello").unwrap(); // 5 bytes
        fs::write(dir.join("sub/b.txt"), "world!").unwrap(); // 6 bytes

        assert_eq!(dir_size(&dir), 11);
    }

    #[test]
    fn human_size_formats() {
        assert_eq!(human_size(500), "500 B");
        assert_eq!(human_size(2048), "2 KiB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }
}
