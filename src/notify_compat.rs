//! Parameters of the legacy `/notify` endpoint.

use serde_json::{Map, Value};

use crate::sink::Notification;

pub const MAX_MESSAGE: usize = 4 * 1024;
pub const MAX_OTHER: usize = 1024;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Params {
    pub message: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub sound: Option<String>,
}

impl Params {
    /// Reads the first value of each key. Empty values count as unset.
    pub fn from_query(query: &str) -> Self {
        let mut params = Self::default();
        for (key, value) in form_urlencoded::parse(query.as_bytes()) {
            let slot = match key.as_ref() {
                "message" => &mut params.message,
                "title" => &mut params.title,
                "subtitle" => &mut params.subtitle,
                "sound" => &mut params.sound,
                _ => continue,
            };
            if slot.is_none() {
                *slot = Some(value.into_owned());
            }
        }
        params.drop_empty()
    }

    /// Unknown keys are ignored; known keys must be strings or null.
    pub fn from_json(map: &Map<String, Value>) -> Result<Self, String> {
        let field = |name: &str| match map.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("{name} must be a string")),
        };
        Ok(Self {
            message: field("message")?,
            title: field("title")?,
            subtitle: field("subtitle")?,
            sound: field("sound")?,
        }
        .drop_empty())
    }

    /// Query values win over JSON values.
    pub fn merge(query: Self, json: Self) -> Self {
        Self {
            message: query.message.or(json.message),
            title: query.title.or(json.title),
            subtitle: query.subtitle.or(json.subtitle),
            sound: query.sound.or(json.sound),
        }
    }

    pub fn into_notification(self, default_title: &str) -> Result<Notification, String> {
        let message = self.message.ok_or("message is required")?;
        check_len("message", &message, MAX_MESSAGE)?;
        let title = self.title.unwrap_or_else(|| default_title.to_string());
        let subtitle = self.subtitle.unwrap_or_default();
        let sound = self.sound.unwrap_or_default();
        check_len("title", &title, MAX_OTHER)?;
        check_len("subtitle", &subtitle, MAX_OTHER)?;
        check_len("sound", &sound, MAX_OTHER)?;
        Ok(Notification {
            message,
            title,
            subtitle,
            sound,
            thread: String::new(),
            state: None,
        })
    }

    fn drop_empty(self) -> Self {
        let keep = |v: Option<String>| v.filter(|s| !s.is_empty());
        Self {
            message: keep(self.message),
            title: keep(self.title),
            subtitle: keep(self.subtitle),
            sound: keep(self.sound),
        }
    }
}

fn check_len(name: &str, value: &str, max: usize) -> Result<(), String> {
    if value.len() > max {
        Err(format!("{name} is too long (max {max} bytes)"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(s: &str) -> Map<String, Value> {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn query_decodes_and_takes_first_value() {
        let p = Params::from_query("message=hello%20world&title=a+b&message=second&x=1");
        assert_eq!(p.message.as_deref(), Some("hello world"));
        assert_eq!(p.title.as_deref(), Some("a b"));
    }

    #[test]
    fn empty_values_are_unset() {
        let p = Params::from_query("message=&title=");
        assert_eq!(p, Params::default());
        let p = Params::from_json(&json(r#"{"message":"","title":null}"#)).unwrap();
        assert_eq!(p, Params::default());
    }

    #[test]
    fn json_rejects_non_strings_and_ignores_unknown() {
        assert!(Params::from_json(&json(r#"{"message":1}"#)).is_err());
        assert!(Params::from_json(&json(r#"{"sound":["a"]}"#)).is_err());
        let p = Params::from_json(&json(r#"{"message":"m","extra":{"a":1}}"#)).unwrap();
        assert_eq!(p.message.as_deref(), Some("m"));
    }

    #[test]
    fn query_wins_and_empty_query_falls_back_to_json() {
        let q = Params::from_query("title=from-query&message=");
        let j = Params::from_json(&json(r#"{"title":"from-json","message":"m"}"#)).unwrap();
        let merged = Params::merge(q, j);
        assert_eq!(merged.title.as_deref(), Some("from-query"));
        assert_eq!(merged.message.as_deref(), Some("m"));
    }

    #[test]
    fn default_title_and_empty_optionals() {
        let n = Params::from_query("message=hi")
            .into_notification("Default")
            .unwrap();
        assert_eq!(
            n,
            Notification {
                message: "hi".into(),
                title: "Default".into(),
                subtitle: String::new(),
                sound: String::new(),
                thread: String::new(),
                state: None,
            }
        );
    }

    #[test]
    fn message_required() {
        assert!(Params::default().into_notification("t").is_err());
    }

    #[test]
    fn length_limits_are_bytes() {
        let at = |field: &str, len: usize| {
            let mut p = Params {
                message: Some("m".into()),
                ..Params::default()
            };
            let v = Some("あ".repeat(len / 3) + &"a".repeat(len % 3));
            match field {
                "message" => p.message = v,
                "title" => p.title = v,
                "subtitle" => p.subtitle = v,
                _ => p.sound = v,
            }
            p.into_notification("t")
        };
        for (field, max) in [
            ("message", MAX_MESSAGE),
            ("title", MAX_OTHER),
            ("subtitle", MAX_OTHER),
            ("sound", MAX_OTHER),
        ] {
            assert!(at(field, max).is_ok(), "{field} at limit");
            assert!(at(field, max + 1).is_err(), "{field} over limit");
        }
    }
}
