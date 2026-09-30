//! Registering qview with Windows for its image types, per user (under
//! HKEY_CURRENT_USER, no administrator rights):
//!
//! - a ProgID per format, `qview.<id>` (`filetypes::FILE_TYPES`), with its
//!   name, its icon from the executable and the open command;
//! - the ProgID in each extension's `OpenWithProgids`, so that qview is
//!   offered in "Open with" and Windows asks, on the next open, whether it
//!   should be used from now on;
//! - `Applications\qview.exe` and `Capabilities` listed in
//!   `RegisteredApplications`, so that qview appears in Settings → Default
//!   apps.
//!
//! Windows does not let a program make itself the default: the choice
//! (`UserChoice`) is protected and belongs to the user. So `register` only
//! offers qview, and `open_default_apps` opens the page where the user
//! picks it.

use std::path::Path;

use crate::filetypes::{FILE_TYPES, icon_id};

/// Where the keys go, under HKEY_CURRENT_USER. Tests use a scratch key.
struct Roots {
    classes: String,
    software: String,
    registered_apps: String,
}

impl Roots {
    fn real() -> Self {
        Self {
            classes: r"Software\Classes".into(),
            software: r"Software\qview".into(),
            registered_apps: r"Software\RegisteredApplications".into(),
        }
    }
}

const APP_NAME: &str = "qview";
const APP_KEY: &str = r"Applications\qview.exe";

fn prog_id(id: &str) -> String {
    format!("qview.{id}")
}

/// The command Explorer runs to open a file with `exe`.
fn open_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    NotRegistered,
    /// Registered for this executable.
    Registered,
    /// Registered for another copy, at this path.
    OtherCopy(String),
}

/// Register `exe` (the running qview) for all its image types.
pub fn register(exe: &Path) -> Result<(), String> {
    register_in(&Roots::real(), exe)?;
    notify_shell();
    Ok(())
}

/// Remove everything `register` wrote. A type whose default was qview goes
/// back to asking which program to use.
pub fn unregister() -> Result<(), String> {
    unregister_in(&Roots::real())?;
    notify_shell();
    Ok(())
}

/// Whether qview is registered, and for which copy.
pub fn status(exe: &Path) -> Status {
    status_in(&Roots::real(), exe)
}

/// For each of `FILE_TYPES`: whether the user has chosen qview as the
/// default program for its main extension.
pub fn defaults() -> Vec<bool> {
    FILE_TYPES
        .iter()
        .map(|t| {
            let key = format!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.{}\UserChoice", t.extensions[0]);
            reg::get(&key, Some("ProgId")).is_some_and(|p| p.eq_ignore_ascii_case(&prog_id(t.id)))
        })
        .collect()
}

/// Open Settings → Default apps (on Windows 11 at qview's own page).
pub fn open_default_apps() -> bool {
    crate::win::shell_open("ms-settings:defaultapps?registeredAppUser=qview")
}

fn register_in(r: &Roots, exe: &Path) -> Result<(), String> {
    let command = open_command(exe);
    let exe_text = exe.display();
    let ru = crate::i18n::lang() == crate::i18n::Lang::Ru;
    let capabilities = format!(r"{}\Capabilities", r.software);
    for (i, t) in FILE_TYPES.iter().enumerate() {
        let id = prog_id(t.id);
        let key = format!(r"{}\{id}", r.classes);
        reg::set(&key, None, if ru { t.name_ru } else { t.name_en })?;
        reg::set(&format!(r"{key}\DefaultIcon"), None, &format!("\"{exe_text}\",-{}", icon_id(i)))?;
        reg::set(&format!(r"{key}\shell\open\command"), None, &command)?;
        for ext in t.extensions {
            reg::set(&format!(r"{}\.{ext}\OpenWithProgids", r.classes), Some(&id), "")?;
            reg::set(&format!(r"{}\{APP_KEY}\SupportedTypes", r.classes), Some(&format!(".{ext}")), "")?;
            reg::set(&format!(r"{capabilities}\FileAssociations"), Some(&format!(".{ext}")), &id)?;
        }
    }
    let app = format!(r"{}\{APP_KEY}", r.classes);
    reg::set(&app, Some("FriendlyAppName"), APP_NAME)?;
    reg::set(&format!(r"{app}\shell\open\command"), None, &command)?;
    reg::set(&capabilities, Some("ApplicationName"), APP_NAME)?;
    reg::set(
        &capabilities,
        Some("ApplicationDescription"),
        if ru { "Быстрый и простой просмотрщик изображений" } else { "Fast and simple image viewer" },
    )?;
    reg::set(&capabilities, Some("ApplicationIcon"), &format!("\"{exe_text}\",0"))?;
    reg::set(&r.registered_apps, Some(APP_NAME), &capabilities)?;
    Ok(())
}

fn unregister_in(r: &Roots) -> Result<(), String> {
    for t in FILE_TYPES {
        let id = prog_id(t.id);
        reg::delete_tree(&format!(r"{}\{id}", r.classes))?;
        for ext in t.extensions {
            reg::delete_value(&format!(r"{}\.{ext}\OpenWithProgids", r.classes), &id)?;
        }
    }
    reg::delete_tree(&format!(r"{}\{APP_KEY}", r.classes))?;
    reg::delete_tree(&r.software)?;
    reg::delete_value(&r.registered_apps, APP_NAME)?;
    Ok(())
}

fn status_in(r: &Roots, exe: &Path) -> Status {
    match reg::get(&format!(r"{}\{APP_KEY}\shell\open\command", r.classes), None) {
        None => Status::NotRegistered,
        Some(c) if c.eq_ignore_ascii_case(&open_command(exe)) => Status::Registered,
        // `"path" "%1"`: the path between the first pair of quotes.
        Some(c) => Status::OtherCopy(c.split('"').nth(1).unwrap_or(&c).to_string()),
    }
}

/// Tell Explorer that associations changed, so that it shows the new
/// icons and names without a restart.
fn notify_shell() {
    use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
}

/// String values under HKEY_CURRENT_USER.
mod reg {
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
    };

    use crate::win::wide;

    fn name_ptr(name: &Option<Vec<u16>>) -> *const u16 {
        name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr())
    }

    fn check(code: u32, what: &str, key: &str) -> Result<(), String> {
        if code == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!("{what} {key}: {}", std::io::Error::from_raw_os_error(code as i32)))
        }
    }

    /// Set value `name` (None: the key's default value) of `key`, creating
    /// the key if needed.
    pub fn set(key: &str, name: Option<&str>, value: &str) -> Result<(), String> {
        let (k, n, v) = (wide(key), name.map(wide), wide(value));
        let code = unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), name_ptr(&n), REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32)
        };
        check(code, "cannot write", key)
    }

    /// Value `name` of `key`, if it is there.
    pub fn get(key: &str, name: Option<&str>) -> Option<String> {
        let (k, n) = (wide(key), name.map(wide));
        let mut buf = vec![0u16; 1024];
        loop {
            let mut bytes = (buf.len() * 2) as u32;
            let code = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    k.as_ptr(),
                    name_ptr(&n),
                    RRF_RT_REG_SZ,
                    std::ptr::null_mut(),
                    buf.as_mut_ptr().cast(),
                    &mut bytes,
                )
            };
            match code {
                ERROR_SUCCESS => {
                    let len = (bytes as usize / 2).saturating_sub(1);
                    return Some(String::from_utf16_lossy(&buf[..len]));
                }
                // ERROR_MORE_DATA: `bytes` holds the size needed.
                234 => buf.resize(bytes as usize / 2 + 1, 0),
                _ => return None,
            }
        }
    }

    /// Delete `key` with everything under it; a missing key is fine.
    pub fn delete_tree(key: &str) -> Result<(), String> {
        let k = wide(key);
        let code = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
        if code == ERROR_FILE_NOT_FOUND { Ok(()) } else { check(code, "cannot delete", key) }
    }

    /// Delete value `name` of `key`; a missing one is fine.
    pub fn delete_value(key: &str, name: &str) -> Result<(), String> {
        let (k, n) = (wide(key), wide(name));
        let code = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr()) };
        if code == ERROR_FILE_NOT_FOUND { Ok(()) } else { check(code, "cannot delete", key) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Registration into a scratch key, never the real associations.
    #[test]
    fn register_and_unregister() {
        let root = format!(r"Software\qview_test_{}", std::process::id());
        let r = Roots {
            classes: format!(r"{root}\Classes"),
            software: format!(r"{root}\Software\qview"),
            registered_apps: format!(r"{root}\RegisteredApplications"),
        };
        let exe = Path::new(r"C:\Program Files\qview\qview.exe");
        assert_eq!(status_in(&r, exe), Status::NotRegistered);
        register_in(&r, exe).unwrap();
        assert_eq!(status_in(&r, exe), Status::Registered);
        assert_eq!(
            status_in(&r, Path::new(r"D:\other\qview.exe")),
            Status::OtherCopy(r"C:\Program Files\qview\qview.exe".into())
        );
        let get = |key: &str, name: Option<&str>| reg::get(&format!(r"{}\{key}", r.classes), name);
        assert_eq!(get(r"qview.jpeg\shell\open\command", None).unwrap(), r#""C:\Program Files\qview\qview.exe" "%1""#);
        assert_eq!(get(r"qview.jpeg\DefaultIcon", None).unwrap(), r#""C:\Program Files\qview\qview.exe",-101"#);
        assert_eq!(get(r".jfif\OpenWithProgids", Some("qview.jpeg")).unwrap(), "");
        assert_eq!(get(r".pgm\OpenWithProgids", Some("qview.pnm")).unwrap(), "");
        assert_eq!(
            reg::get(&format!(r"{}\Capabilities\FileAssociations", r.software), Some(".tif")).unwrap(),
            "qview.tiff"
        );
        assert_eq!(reg::get(&r.registered_apps, Some("qview")).unwrap(), format!(r"{}\Capabilities", r.software));

        // Another program's entry in OpenWithProgids survives.
        reg::set(&format!(r"{}\.png\OpenWithProgids", r.classes), Some("Other.png"), "").unwrap();
        unregister_in(&r).unwrap();
        assert_eq!(status_in(&r, exe), Status::NotRegistered);
        assert_eq!(get("qview.jpeg", None), None);
        assert_eq!(get(r".png\OpenWithProgids", Some("qview.png")), None);
        assert_eq!(get(r".png\OpenWithProgids", Some("Other.png")).unwrap(), "");
        assert_eq!(reg::get(&r.registered_apps, Some("qview")), None);
        // Unregistering twice is fine.
        unregister_in(&r).unwrap();
        reg::delete_tree(&root).unwrap();
    }

    #[test]
    fn every_listed_extension_is_registered() {
        let mut registered: Vec<&str> = FILE_TYPES.iter().flat_map(|t| t.extensions.iter().copied()).collect();
        let mut listed = crate::folder::EXTENSIONS.to_vec();
        registered.sort_unstable();
        listed.sort_unstable();
        assert_eq!(registered, listed);
    }
}
