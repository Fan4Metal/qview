//! Interface language: Russian when Windows shows its interface in Russian,
//! English otherwise. There is no setting for it.
//!
//! Strings stay next to the code that shows them, both languages together:
//! `tr!("Delete", "Удалить")` gives the one of the current language (and
//! evaluates only that one, so `format!` arms cost nothing extra).

use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    Ru,
}

static LANG: AtomicU8 = AtomicU8::new(0);

/// The language the interface is shown in.
pub fn lang() -> Lang {
    match LANG.load(Relaxed) {
        1 => Lang::Ru,
        _ => Lang::En,
    }
}

pub fn set_lang(lang: Lang) {
    LANG.store(lang as u8, Relaxed);
}

/// Russian when Windows shows its interface in Russian, else English.
pub fn system_lang() -> Lang {
    if crate::win::ui_language_is_russian() {
        Lang::Ru
    } else {
        Lang::En
    }
}

/// The English or the Russian expression, by the current language.
#[macro_export]
macro_rules! tr {
    ($en:expr, $ru:expr $(,)?) => {
        match $crate::i18n::lang() {
            $crate::i18n::Lang::En => $en,
            $crate::i18n::Lang::Ru => $ru,
        }
    };
}
