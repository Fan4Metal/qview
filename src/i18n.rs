//! Interface language: English or Russian, taken from the language of the
//! Windows interface unless chosen in the About window (as in
//! disk_flashlight).
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

/// The language as kept in the settings: a fixed one, or the system's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LangChoice {
    #[default]
    System,
    En,
    Ru,
}

impl LangChoice {
    pub const ALL: [LangChoice; 3] = [LangChoice::System, LangChoice::En, LangChoice::Ru];

    pub fn resolve(self) -> Lang {
        match self {
            LangChoice::System => system_lang(),
            LangChoice::En => Lang::En,
            LangChoice::Ru => Lang::Ru,
        }
    }

    /// Name in the settings.
    pub fn name(self) -> &'static str {
        match self {
            LangChoice::System => "system",
            LangChoice::En => "en",
            LangChoice::Ru => "ru",
        }
    }

    pub fn from_name(name: &str) -> Option<LangChoice> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }

    /// As listed in the About window: the languages in their own language,
    /// so that they can be found whatever the interface shows.
    pub fn label(self) -> String {
        let own = |lang| match lang {
            Lang::En => "English",
            Lang::Ru => "Русский",
        };
        match self {
            LangChoice::System => {
                let own = own(system_lang());
                crate::tr!(format!("As in Windows ({own})"), format!("Как в Windows ({own})"))
            }
            LangChoice::En | LangChoice::Ru => own(self.resolve()).into(),
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

/// The language the interface is shown in now.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_names_round_trip() {
        for c in LangChoice::ALL {
            assert_eq!(LangChoice::from_name(c.name()), Some(c));
        }
        assert_eq!(LangChoice::from_name("de"), None);
        assert_eq!(LangChoice::Ru.label(), "Русский");
    }
}
