use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let root_dir = manifest_dir
        .parent()
        .expect("kernel has project root parent");
    let userland_dir = root_dir.join("target/userland");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    println!("cargo:rerun-if-changed={}", userland_dir.display());
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    println!("cargo:rerun-if-env-changed=VANTARA_BUILD_TIMESTAMP");
    println!("cargo:rerun-if-env-changed=VANTARA_GIT_COMMIT");
    println!("cargo:rerun-if-env-changed=VANTARA_GIT_DIRTY");
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join(".git/HEAD").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join(".git/index").display()
    );

    let mut bins = collect_bins(&userland_dir);
    bins.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| image_priority(&right.1).cmp(&image_priority(&left.1)))
    });
    bins.dedup_by(|left, right| left.0 == right.0);

    let mut images = String::from("pub static USERLAND_IMAGES: &[UserlandImage] = &[\n");
    let mut names = String::from("pub static USERLAND_IMAGE_NAMES: &[&str] = &[\n");

    for (name, path) in bins {
        let bin_path = format!("/bin/{name}");
        let file_name = path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .expect("userland artifact has UTF-8 file name");
        images.push_str(&format!(
            "    UserlandImage {{ path: \"{bin_path}\", name: \"{name}\", data: include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../target/userland/{file_name}\")) }},\n"
        ));
        names.push_str(&format!("    \"{}\",\n", name));
        println!("cargo:rerun-if-changed={}", path.display());
    }

    images.push_str("];\n");
    names.push_str("];\n");

    let registry = format!("{images}\n{names}");
    fs::write(out_dir.join("userland_images.rs"), &registry)
        .expect("write generated userland image registry");

    let generated_dir = root_dir.join("target/generated");
    fs::create_dir_all(&generated_dir).expect("create stable generated artifact directory");
    fs::write(generated_dir.join("userland_images.rs"), registry)
        .expect("write stable generated userland image registry");

    write_build_metadata(&manifest_dir, &out_dir, &generated_dir);
}

fn write_build_metadata(manifest_dir: &Path, out_dir: &Path, generated_dir: &Path) {
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".into());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "unknown".into());
    let git_commit = env::var("VANTARA_GIT_COMMIT")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| git_output(manifest_dir, &["rev-parse", "--short=12", "HEAD"]))
        .unwrap_or_else(|| "unknown".into());
    let git_dirty = env::var("VANTARA_GIT_DIRTY")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| detect_git_dirty(manifest_dir, &git_commit));
    let build_timestamp = env::var("VANTARA_BUILD_TIMESTAMP")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            env::var("SOURCE_DATE_EPOCH")
                .ok()
                .filter(|value| !value.is_empty())
                .map(|epoch| format_source_date_epoch(&epoch))
        })
        .unwrap_or_else(|| "unknown".into());

    let rust_metadata = format!(
        "pub const VERSION: &str = {version:?};\n\
         pub const PROFILE: &str = {profile:?};\n\
         pub const GIT_COMMIT: &str = {git_commit:?};\n\
         pub const GIT_DIRTY: &str = {git_dirty:?};\n\
         pub const BUILD_TIMESTAMP: &str = {build_timestamp:?};\n"
    );
    fs::write(out_dir.join("build_metadata.rs"), rust_metadata)
        .expect("write generated Rust build metadata");

    let artifact_metadata = format!(
        "key\tvalue\n\
         version\t{version}\n\
         profile\t{profile}\n\
         git_commit\t{git_commit}\n\
         git_dirty\t{git_dirty}\n\
         build_timestamp\t{build_timestamp}\n"
    );
    fs::write(generated_dir.join("build-metadata.tsv"), artifact_metadata)
        .expect("write stable build metadata artifact");
}

fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn detect_git_dirty(cwd: &Path, git_commit: &str) -> String {
    if git_commit == "unknown" {
        return "unknown".into();
    }

    match Command::new("git")
        .current_dir(cwd)
        .args(["status", "--porcelain"])
        .output()
    {
        Ok(output) if output.status.success() && output.stdout.is_empty() => "false".into(),
        Ok(output) if output.status.success() => "true".into(),
        _ => "unknown".into(),
    }
}

fn format_source_date_epoch(epoch: &str) -> String {
    let argument = format!("@{epoch}");
    let output = Command::new("date")
        .args(["-u", "-d", &argument, "+%Y-%m-%dT%H:%M:%SZ"])
        .output();

    match output {
        Ok(output) if output.status.success() => String::from_utf8(output.stdout)
            .map(|value| value.trim().to_owned())
            .unwrap_or_else(|_| format!("unix:{epoch}")),
        _ => format!("unix:{epoch}"),
    }
}

fn is_elf_path(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("elf")
}

fn image_priority(path: &Path) -> u8 {
    if is_elf_path(path) { 1 } else { 0 }
}

fn collect_bins(userland_dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(userland_dir) else {
        return Vec::new();
    };

    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let extension = path.extension().and_then(|ext| ext.to_str());
            if extension != Some("bin") && extension != Some("elf") {
                return None;
            }

            let name = path.file_stem()?.to_str()?.to_owned();
            Some((name, path))
        })
        .collect()
}
