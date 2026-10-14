use crate::{Failure, Result, http::Credentials, pytext};
use std::{collections::HashMap, path::Path};

pub struct Environment {
    dotenv: HashMap<String, String>,
    #[cfg(test)]
    isolated: bool,
}

impl Environment {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => {
                return Ok(Self {
                    dotenv: HashMap::new(),
                    #[cfg(test)]
                    isolated: false,
                });
            }
        };
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
        let text = std::str::from_utf8(bytes).map_err(|_| {
            Failure::message(format!(
                "error: {} is not valid UTF-8; save it as UTF-8 and re-run.",
                crate::pypath::text(path)
            ))
        })?;
        let mut dotenv = HashMap::new();
        for raw in text.split([
            '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}',
            '\u{2029}',
        ]) {
            let line = pytext::strip(raw);
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = pytext::strip(key);
            let mut value = pytext::strip(value);
            if value.len() >= 2
                && ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')))
            {
                value = &value[1..value.len() - 1];
            }
            if !key.is_empty() {
                let key = if cfg!(windows) {
                    key.to_ascii_uppercase()
                } else {
                    key.into()
                };
                dotenv.entry(key).or_insert_with(|| value.to_string());
            }
        }
        Ok(Self {
            dotenv,
            #[cfg(test)]
            isolated: false,
        })
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let dotenv_key = if cfg!(windows) {
            key.to_ascii_uppercase()
        } else {
            key.into()
        };
        #[cfg(test)]
        if self.isolated {
            return self.dotenv.get(&dotenv_key).cloned();
        }
        std::env::var_os(key)
            .map(|v| crate::pypath::decode_os(&v))
            .or_else(|| self.dotenv.get(&dotenv_key).cloned())
    }

    pub fn credentials(&self) -> Result<Credentials> {
        match (self.get("XERO_CLIENT_ID"), self.get("XERO_CLIENT_SECRET")) {
            (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => {
                Ok(Credentials { id, secret })
            }
            _ => Err(Failure::message(
                "Set XERO_CLIENT_ID and XERO_CLIENT_SECRET in .env (see .env.example).",
            )),
        }
    }

    pub fn has_dotenv_proxy(&self) -> bool {
        self.dotenv
            .keys()
            .any(|key| key.to_ascii_lowercase().ends_with("_proxy") || key == "REQUEST_METHOD")
    }

    #[cfg(test)]
    pub(crate) fn isolated() -> Self {
        Self {
            dotenv: HashMap::from([("no_proxy".into(), "*".into())]),
            isolated: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_dotenv_is_optional() {
        let environment = Environment::load(Path::new("missing-synthetic-dotenv-file")).unwrap();
        assert!(environment.dotenv.is_empty());
    }
    #[test]
    fn synthetic_dotenv_preserves_first_value_quotes_bom_and_platform_key_case() {
        let scratch = crate::testing::Scratch::new();
        let file = scratch.0.join("synthetic-settings");
        std::fs::write(&file,"\u{feff}# synthetic settings\nXERO_SYNTHETIC_ONLY = 'first'\nXERO_SYNTHETIC_ONLY=second\nxero_synthetic_lower=lower\n").unwrap();
        let environment = Environment::load(&file).unwrap();
        assert_eq!(
            environment.get("XERO_SYNTHETIC_ONLY").as_deref(),
            Some("first")
        );
        assert_eq!(
            environment.get("XERO_SYNTHETIC_LOWER").as_deref(),
            if cfg!(windows) { Some("lower") } else { None }
        );
        std::fs::write(&file, [0xff, 0xfe, 0]).unwrap();
        assert!(Environment::load(&file).is_err());
    }
}
