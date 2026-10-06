//! Open in Editor: the programs Windows offers for a file type (those of
//! "Open with", `SHAssocEnumHandlers`), or any other program the user picks.
//! The one chosen last is kept in the settings and opened by Ctrl+E.
//!
//! A program is known by its handler's name: the path of its executable,
//! or a Store app's identity. Opening goes through Windows' handler of that
//! name for the file type when there is one, since a Store app (the
//! Windows 11 Paint, Photos) starts no other way; a program the user
//! picked is run with the files as arguments. The COM interfaces are
//! declared by hand, as in `wic`.

use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use windows_sys::core::{GUID, HRESULT, PWSTR};

use crate::wic::{Com, Unknown};
use crate::win::wide;

use windows_sys::Win32::Foundation::{E_FAIL, E_INVALIDARG};

const IID_IDATAOBJECT: GUID = GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);

/// `IEnumAssocHandlers::Next`.
type Next = unsafe extern "system" fn(Unknown, u32, *mut Unknown, *mut u32) -> HRESULT;
/// `IAssocHandler::GetName` and `GetUIName`.
type GetString = unsafe extern "system" fn(Unknown, *mut PWSTR) -> HRESULT;
/// `IAssocHandler::Invoke`.
type Invoke = unsafe extern "system" fn(Unknown, Unknown) -> HRESULT;
/// `IShellItemArray::BindToHandler`.
type BindToHandler = unsafe extern "system" fn(Unknown, Unknown, *const GUID, *const GUID, *mut Unknown) -> HRESULT;

// Vtable indices, from shobjidl_core.h.
const ENUM_NEXT: usize = 3;
const HANDLER_GET_NAME: usize = 3;
const HANDLER_GET_UI_NAME: usize = 4;
const HANDLER_INVOKE: usize = 8;
const ARRAY_BIND_TO_HANDLER: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editor {
    /// The handler's name: an executable's path, or a Store app's identity.
    pub id: String,
    /// As Windows shows it ("Paint").
    pub name: String,
}

impl Editor {
    /// A program the user picked: named after its file.
    pub fn program(path: &Path) -> Editor {
        let name = path.file_stem().unwrap_or(path.as_os_str()).to_string_lossy().into_owned();
        Editor { id: path.to_string_lossy().into_owned(), name }
    }
}

/// Windows' recommended handlers for the extension `ext` (".jpg"), with
/// their interfaces. COM is initialised by the caller.
fn handlers(ext: &str) -> Vec<(Editor, Com)> {
    use windows_sys::Win32::UI::Shell::{ASSOC_FILTER_RECOMMENDED, SHAssocEnumHandlers};
    let mut handlers = Vec::new();
    let mut found = null_mut();
    let ext = wide(ext);
    if unsafe { SHAssocEnumHandlers(ext.as_ptr(), ASSOC_FILTER_RECOMMENDED, &mut found) } < 0 || found.is_null() {
        return handlers;
    }
    let found = Com(found);
    loop {
        let (mut handler, mut fetched) = (null_mut(), 0u32);
        let hr = unsafe { found.method::<Next>(ENUM_NEXT)(found.0, 1, &mut handler, &mut fetched) };
        // S_FALSE (1) when there are no more.
        if hr != 0 || fetched == 0 || handler.is_null() {
            break;
        }
        let handler = Com(handler);
        if let (Some(id), Some(name)) = (string(&handler, HANDLER_GET_NAME), string(&handler, HANDLER_GET_UI_NAME)) {
            handlers.push((Editor { id, name }, handler));
        }
    }
    handlers
}

/// The string a handler's method `index` returns (allocated by COM).
fn string(handler: &Com, index: usize) -> Option<String> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    let mut s: PWSTR = null_mut();
    let hr = unsafe { handler.method::<GetString>(index)(handler.0, &mut s) };
    if hr < 0 || s.is_null() {
        return None;
    }
    let text = unsafe {
        let len = (0..).take_while(|&i| *s.add(i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(s, len))
    };
    unsafe { CoTaskMemFree(s.cast()) };
    Some(text)
}

/// This program, or another copy of it: not offered as an editor.
fn is_qview(id: &str) -> bool {
    Path::new(id).file_name().is_some_and(|n| n.eq_ignore_ascii_case("qview.exe"))
}

/// The programs Windows offers for files with the extension `ext` (".jpg"),
/// by name, qview left out.
pub fn for_extension(ext: &str) -> Vec<Editor> {
    let _com = crate::win::com_init();
    let mut editors: Vec<Editor> = handlers(ext).into_iter().map(|(e, _)| e).filter(|e| !is_qview(&e.id)).collect();
    editors.sort_by_key(|e| e.name.to_lowercase());
    editors.dedup_by(|a, b| a.id.eq_ignore_ascii_case(&b.id));
    editors
}

/// Open `files` in `editor`; with none chosen, with Windows' "edit" verb
/// (Paint for most images). Blocks while the program is started, so call
/// it off the UI thread.
pub fn open(editor: Option<&Editor>, files: &[PathBuf]) -> Result<(), String> {
    let _com = crate::win::com_init();
    let Some(editor) = editor else { return edit_verb(files) };
    let ext = files.first().and_then(|f| f.extension()).map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    if let Some((_, handler)) = handlers(&ext).into_iter().find(|(e, _)| e.id.eq_ignore_ascii_case(&editor.id)) {
        let name = &editor.name;
        // The error, with the HRESULT when the handler refused.
        let invoke = |files: &[PathBuf]| -> Result<(), (HRESULT, String)> {
            let data = data_object(files).map_err(|e| (0, e))?;
            let hr = unsafe { handler.method::<Invoke>(HANDLER_INVOKE)(handler.0, data.0) };
            if hr >= 0 {
                Ok(())
            } else {
                Err((hr, tr!(format!("{name} did not open the file (error {hr:#x})"), format!("{name} не открыл файл (ошибка {hr:#x})"))))
            }
        };
        // Many handlers take one file at a time (E_FAIL for several, as
        // Paint's): then each is opened on its own, the others even if one
        // fails.
        let result = match invoke(files) {
            Err((E_FAIL | E_INVALIDARG, _)) if files.len() > 1 => {
                let results: Vec<_> = files.iter().map(|f| invoke(std::slice::from_ref(f))).collect();
                results.into_iter().find(Result::is_err).unwrap_or(Ok(()))
            }
            result => result,
        };
        return result.map_err(|(_, e)| e);
    }
    if Path::new(&editor.id).is_file() {
        return std::process::Command::new(&editor.id).args(files).spawn().map(|_| ()).map_err(|e| e.to_string());
    }
    let name = &editor.name;
    Err(tr!(format!("{name} does not open files of this type"), format!("{name} не открывает файлы этого типа")))
}

/// `files` as a data object, which a handler is invoked with.
pub fn data_object(files: &[PathBuf]) -> Result<Com, String> {
    use windows_sys::Win32::UI::Shell::{BHID_DataObject, ILCreateFromPathW, ILFree, SHCreateShellItemArrayFromIDLists};
    let ids: Vec<_> = files.iter().map(|f| unsafe { ILCreateFromPathW(wide(f).as_ptr()) }).collect();
    let made = (|| {
        if ids.iter().any(|id| id.is_null()) {
            return Err(tr!("The file is not there", "Файла нет на месте").to_string());
        }
        let mut array = null_mut();
        let hr = unsafe { SHCreateShellItemArrayFromIDLists(ids.len() as u32, ids.as_ptr().cast(), &mut array) };
        if hr < 0 || array.is_null() {
            return Err(tr!(format!("error {hr:#x}"), format!("ошибка {hr:#x}")));
        }
        let array = Com(array);
        let mut data = null_mut();
        let hr = unsafe {
            array.method::<BindToHandler>(ARRAY_BIND_TO_HANDLER)(array.0, null_mut(), &BHID_DataObject, &IID_IDATAOBJECT, &mut data)
        };
        if hr < 0 || data.is_null() {
            return Err(tr!(format!("error {hr:#x}"), format!("ошибка {hr:#x}")));
        }
        Ok(Com(data))
    })();
    for id in ids.into_iter().filter(|id| !id.is_null()) {
        unsafe { ILFree(id) };
    }
    made
}

/// `files` opened with the "edit" verb of their type.
fn edit_verb(files: &[PathBuf]) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let verb = wide("edit");
    for file in files {
        let path = wide(file);
        let result = unsafe {
            ShellExecuteW(null_mut(), verb.as_ptr(), path.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
        };
        // Above 32 on success.
        if result as usize <= 32 {
            return Err(tr!(
                "Windows has no editor for this type: choose one in Edit With",
                "В Windows нет редактора для этого типа: выберите его в «Редактировать в»"
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The editors Windows offers for `QVIEW_EDITOR_EXT` (".jpg" if not
    /// set), printed with `--nocapture`.
    #[test]
    #[ignore]
    fn print_editors() {
        let ext = std::env::var("QVIEW_EDITOR_EXT").unwrap_or(".jpg".into());
        for e in for_extension(&ext) {
            println!("{}  ({})", e.name, e.id);
        }
    }

    #[test]
    fn several_files_make_one_data_object() {
        let _com = crate::win::com_init();
        let dir = std::env::temp_dir().join(format!("qview_editors_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let files: Vec<PathBuf> = (0..3).map(|i| dir.join(format!("{i}.png"))).collect();
        for f in &files {
            std::fs::write(f, b"x").unwrap();
        }
        assert!(data_object(&files[..1]).is_ok());
        assert!(data_object(&files).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn qview_is_left_out() {
        assert!(is_qview(r"C:\Users\x\AppData\Local\Programs\qview\QVIEW.EXE"));
        assert!(!is_qview(r"C:\Windows\system32\mspaint.exe"));
        assert_eq!(Editor::program(Path::new(r"C:\Apps\gimp-2.10.exe")).name, "gimp-2.10");
    }
}
