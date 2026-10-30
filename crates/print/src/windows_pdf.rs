//! Optional Windows PDF printing component. Never called during application startup.
//! MSVC delay-loads the import (see packaging/windows/optional_pdf_link.rs).
//! Use the maintained safe winsafe wrapper; no raw FFI or application-directory DLLs.
#[cfg(any(windows, test))]
fn with_loaded<G, T>(load: impl FnOnce() -> Result<G, String>, operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let _library = load()?;
    operation()
}

pub(crate) fn run<T>(operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    #[cfg(windows)]
    {
        with_loaded(load_system_renderer, operation)
    }
    #[cfg(not(windows))]
    {
        operation()
    }
}

#[cfg(windows)]
fn unavailable(detail: impl std::fmt::Display) -> String {
    format!(
        "Windows PDF 打印组件（windows.data.pdf.dll / prntvpt.dll）不可用（{detail}）。仅打印受影响，文档和 AI 翻译仍可使用。请修复 Windows 系统组件，或导出 PDF 后使用其他软件打印；不要从第三方网站下载 DLL。"
    )
}

#[cfg(windows)]
fn load_system_renderer() -> Result<Vec<winsafe::guard::FreeLibraryGuard>, String> {
    let directory = winsafe::GetSystemDirectory().map_err(unavailable)?;
    let directory = std::path::PathBuf::from(directory);
    let renderer = load_renderer_at(&directory.join("windows.data.pdf.dll"))?;
    let tickets = load_library_at(
        &directory.join("prntvpt.dll"),
        &[
            "PTCloseProvider",
            "PTReleaseMemory",
            "PTMergeAndValidatePrintTicket",
            "PTConvertPrintTicketToDevMode",
            "PTOpenProvider",
            "PTGetPrintCapabilities",
        ],
    )?;
    Ok(vec![renderer, tickets])
}

#[cfg(windows)]
fn load_renderer_at(path: &std::path::Path) -> Result<winsafe::guard::FreeLibraryGuard, String> {
    load_library_at(path, &["PdfCreateRenderer"])
}

#[cfg(windows)]
fn load_library_at(path: &std::path::Path, exports: &[&str]) -> Result<winsafe::guard::FreeLibraryGuard, String> {
    // GetSystemDirectory supplies the native system directory, not a user-controlled env var.
    // The absolute path avoids loading a lookalike beside the executable or in the CWD.
    let path = path.to_str().ok_or_else(|| unavailable("系统路径无法读取"))?;
    let library = winsafe::HINSTANCE::LoadLibrary(path).map_err(|e| unavailable(format!("{path}: {e}")))?;
    for export in exports {
        library.GetProcAddress(export).map_err(|e| unavailable(format!("{path}, {export}: {e}")))?;
    }
    // Hold the module through submission. MSVC's delay helper acquires its own reference
    // on first use; we never unload its resolved IAT or dereference the pointer ourselves.
    Ok(library)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_component_never_enters_native_printing() {
        let called = std::cell::Cell::new(false);
        let result = with_loaded::<(), ()>(
            || Err("missing component".into()),
            || {
                called.set(true);
                Ok(())
            },
        );
        assert_eq!(result, Err("missing component".into()));
        assert!(!called.get());
    }

    #[test]
    fn library_stays_loaded_until_submission_finishes() {
        struct Guard<'a>(&'a std::cell::Cell<bool>);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let loaded = std::cell::Cell::new(true);
        let result = with_loaded(
            || Ok(Guard(&loaded)),
            || {
                assert!(loaded.get());
                Ok("submitted")
            },
        );
        assert_eq!(result, Ok("submitted"));
        assert!(!loaded.get());
    }

    #[test]
    fn native_submission_errors_are_preserved() {
        assert_eq!(with_loaded(|| Ok(()), || Err::<(), _>("printer offline".into())), Err("printer offline".into()));
    }

    #[cfg(windows)]
    #[test]
    fn real_missing_dll_returns_error_without_loading_native_printing() {
        let system = winsafe::GetSystemDirectory().unwrap();
        let missing = std::path::PathBuf::from(system).join("Craft-test-nonexistent-PdfRenderer-20261030.dll");
        let error = load_renderer_at(&missing).err().expect("synthetic DLL must not exist");
        assert!(error.contains("windows.data.pdf.dll") && error.contains("仅打印受影响"));
    }
}
