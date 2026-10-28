//! Mac arm64 packaging (native or cross-built). Personal configuration is never included.
use anyhow::{Context, Result, bail};
use std::{fs, io::Read, path::Path, process::Command};
pub fn run(args: &[String]) -> Result<()> {
    let version = args.first().map(String::as_str).unwrap_or("0.14.0-api.20261029.1");
    if args.len() > 1 || version.is_empty() || !version.bytes().all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c)) {
        bail!("expected one safe package version");
    }
    if !cfg!(target_os = "macos") {
        bail!("local-api-package requires macOS signing tools");
    }
    let root = crate::gates::root();
    let target = std::env::var_os("CARGO_TARGET_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| root.join("target"));
    let binary = std::env::var_os("PDFCRAFT_PACKAGE_BINARY").map(std::path::PathBuf::from).unwrap_or_else(|| target.join("release/pdfcraft"));
    if !binary.is_file() {
        bail!("build cargo build --release -p pdfcraft first");
    }
    let architectures = Command::new("/usr/bin/lipo").arg("-archs").arg(&binary).output()?;
    if !architectures.status.success() || String::from_utf8_lossy(&architectures.stdout).trim() != "arm64" {
        bail!("the macos-arm64 package requires an arm64 Mach-O executable");
    }
    let output = root.join("dist");
    fs::create_dir_all(&output)?;
    let archive = output.join(format!("PdfCraft-{version}-macos-arm64.zip"));
    if archive.exists() {
        bail!("refusing to overwrite {}", archive.display());
    }
    // Keep the transient .app out of indexed/synced Documents folders. Such
    // watchers can reattach Finder metadata between xattr cleanup and signing.
    let staging = std::env::temp_dir().join(format!("pdfcraft-local-api-package-{}", std::process::id()));
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
        fs::copy(root.join("assets/pcb-glossary-public.json"), resources.join("pcb-glossary-public.json"))?;
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
        let models = resources.join("models");
        // An explicitly supplied offline cache may be an existing installation.
        // Only manifest-named files whose exact hashes match are copied; keys,
        // settings and unrelated app resources are never included.
        if let Some(cache) = std::env::var_os("PDFCRAFT_MODELS_DIR") {
            fs::create_dir(&models)?;
            let manifest = crate::assets::load(&root)?;
            for model in manifest.fetched.iter().filter(|m| m.kind == "model") {
                for (file, hash) in
                    [(model.file.clone(), model.sha256.as_str()), (format!("{}.LICENCE.txt", model.file), model.licence_sha256.as_str())]
                {
                    let path = Path::new(&cache).join(&file);
                    let bytes = verified_bytes(fs::File::open(&path).with_context(|| format!("reading {}", path.display()))?, hash, 64 << 20)?;
                    fs::write(models.join(file), bytes)?;
                }
            }
        }
        // Re-verifies every model/license and generates the required attribution.
        crate::assets::models(&[models.to_string_lossy().into_owned()])?;
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
fn verified_bytes(reader: impl Read, sha256: &str, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit || crate::assets::sha256_hex(&bytes) != sha256 {
        bail!("cached model/license is oversized or does not match the pinned manifest hash");
    }
    Ok(bytes)
}
fn check(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if !status.success() {
        bail!("packaging command failed: {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_resources_are_bounded_and_hash_verified_before_packaging() {
        let hash = crate::assets::sha256_hex(b"test-model");
        assert_eq!(verified_bytes(&b"test-model"[..], &hash, 10).unwrap(), b"test-model");
        assert!(verified_bytes(&b"changed"[..], &hash, 10).is_err());
        assert!(verified_bytes(&b"test-model"[..], &hash, 9).is_err());
    }
}
