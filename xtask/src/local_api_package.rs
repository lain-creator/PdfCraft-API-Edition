//! Local Mac-only packaging. No installer registrations, user files, keys or glossary data.
use anyhow::{Context, Result, bail};
use std::{fs, path::Path, process::Command};
pub fn run(args: &[String]) -> Result<()> {
    let version = args.first().map(String::as_str).unwrap_or("0.14.0-api.20261028.1");
    if args.len() > 1 || version.is_empty() || !version.bytes().all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c)) {
        bail!("expected one safe package version");
    }
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        bail!("local-api-package currently supports native macOS arm64 only");
    }
    let root = crate::gates::root();
    let target = std::env::var_os("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| root.join("target"));
    let binary = target.join("release/pdfcraft");
    if !binary.is_file() {
        bail!("build cargo build --release -p pdfcraft first");
    }
    let output = root.join("dist");
    fs::create_dir_all(&output)?;
    let archive = output.join(format!("PdfCraft-{version}-macos-arm64.zip"));
    if archive.exists() {
        bail!("refusing to overwrite {}", archive.display());
    }
    let staging = output.join(format!(".local-api-package-{}", std::process::id()));
    // create_dir refuses an existing name: cleanup below owns only this newly created tree.
    fs::create_dir(&staging)?;
    let result = (|| -> Result<()> {
        let app = staging.join("PdfCraft.app");
        let contents = app.join("Contents");
        let exe = contents.join("MacOS/PdfCraft");
        let resources = contents.join("Resources");
        fs::create_dir_all(exe.parent().context("executable directory")?)?;
        fs::create_dir(&resources)?;
        fs::copy(binary, &exe)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&exe, fs::Permissions::from_mode(0o755))?;
        }
        fs::copy(root.join("assets/app-icon/pdfcraft.icns"), resources.join("PdfCraft.icns"))?;
        let commit = Command::new("git").current_dir(&root).args(["rev-parse", "HEAD"]).output()?;
        if !commit.status.success() {
            bail!("could not determine source commit");
        }
        let sha = String::from_utf8(commit.stdout)?;
        let template = fs::read_to_string(root.join("packaging/macos/Info.plist.in"))?;
        let info = template.replace("@SHORT_VERSION@", "0.14.0").replace("@VERSION@", version).replace("@BUILD_SHA@", sha.trim());
        fs::write(contents.join("Info.plist"), info)?;
        fs::write(contents.join("PkgInfo"), b"APPL????")?;
        for name in ["LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "ATTRIBUTION.md"] {
            fs::copy(root.join(name), resources.join(name))?;
        }
        fs::copy(root.join("docs/local-api-translation.md"), resources.join("API翻译使用说明.md"))?;
        let fonts = std::env::var_os("CRAFT_FONTS_DIR").context("CRAFT_FONTS_DIR is required for the local Chinese edition")?;
        let fonts = Path::new(&fonts);
        fs::copy(fonts.join("ATTRIBUTION.md"), resources.join("craft-fonts-ATTRIBUTION.md"))?;
        for entry in fs::read_dir(fonts.join("fonts"))? {
            let entry = entry?;
            let license = entry.path().join("OFL.txt");
            if license.is_file() {
                fs::copy(license, resources.join(format!("OFL-{}.txt", entry.file_name().to_string_lossy())))?;
            }
        }
        for entry in fs::read_dir(root.join("assets/fonts"))? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with("OFL") {
                fs::copy(entry.path(), resources.join(entry.file_name()))?;
            }
        }
        // Use the upstream manifest's pinned model/license downloader, never
        // unverified resources copied from a previous installed application.
        crate::assets::models(&[resources.join("models").to_string_lossy().into_owned()])?;
        check(Command::new("/usr/bin/xattr").args(["-cr"]).arg(&app))?;
        check(Command::new("/usr/bin/codesign").args(["--force", "--deep", "--sign", "-"]).arg(&app))?;
        check(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(&app))?;
        check(Command::new("/usr/bin/ditto").args(["-c", "-k", "--keepParent", "--norsrc", "--noextattr"]).arg(&app).arg(&archive))?;
        println!("{}", archive.display());
        Ok(())
    })();
    let cleaned = fs::remove_dir_all(&staging);
    result?;
    cleaned?;
    Ok(())
}
fn check(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if !status.success() {
        bail!("packaging command failed: {status}");
    }
    Ok(())
}
