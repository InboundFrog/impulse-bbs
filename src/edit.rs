//! One field editor for every SysOp editor (users, boards, file bases,
//! protocols, menus, system configuration): records go through
//! `serde_json::Value`, so any serde type can be edited field by field.
//! This replaces the per-record screens of SYSOP2*.PAS / SYSOP3.PAS.

use crate::session::Session;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::io;

/// Entry templates for list fields, by field name.
pub type Tmpls = [(String, Value)];

/// Template entry for list field `name`.
pub fn tmpl<T: Serialize + Default>(name: &str) -> (String, Value) {
    (name.to_string(), serde_json::to_value(T::default()).unwrap_or_default())
}

/// A display name for a list entry: its `name`, `keys` or first string field.
fn label(v: &Value) -> String {
    let Some(o) = v.as_object() else { return short(v) };
    for k in ["name", "title", "keys", "descr", "desc"] {
        if let Some(s) = o.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            return if k == "keys" { format!("{s}  {}", o.get("cmd").and_then(|v| v.as_str()).unwrap_or("")) } else { s.to_string() };
        }
    }
    o.values().find_map(|v| v.as_str().map(String::from)).unwrap_or_default()
}

fn short(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => if *b { "Yes" } else { "No" }.into(),
        Value::Array(a) if a.iter().any(|v| v.is_object()) => format!("[{} entries]", a.len()),
        Value::Object(_) => "[...]".into(),
        v => v.to_string(),
    }
}

/// Same shape, emptied: the template for a newly inserted entry.
fn blank(v: &Value) -> Value {
    match v {
        Value::String(_) => Value::String(String::new()),
        Value::Number(_) => 0.into(),
        Value::Bool(_) => false.into(),
        Value::Array(_) => Value::Array(vec![]),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), blank(v))).collect()),
        Value::Null => Value::Null,
    }
}

impl Session {
    /// Edit a typed record. Returns the new value if it was changed and still valid.
    /// `tmpls` gives the entry template for list fields (by field name).
    pub fn edit_rec<T: Serialize + DeserializeOwned>(&mut self, title: &str, rec: &T, hide: &[&str], tmpls: &Tmpls) -> io::Result<Option<T>> {
        let mut v = serde_json::to_value(rec).map_err(io::Error::other)?;
        let hidden: Vec<(String, Value)> = hide
            .iter()
            .filter_map(|k| v.as_object_mut()?.remove(*k).map(|x| (k.to_string(), x)))
            .collect();
        if !self.edit_value(title, &mut v, None, tmpls)? {
            return Ok(None);
        }
        if let Some(o) = v.as_object_mut() {
            o.extend(hidden);
        }
        match serde_json::from_value(v) {
            Ok(t) => Ok(Some(t)),
            Err(e) => {
                self.sprint(&format!("|12Not saved: {e}"))?;
                Ok(None)
            }
        }
    }

    /// Edit a typed list; `template` is the starting point for inserted entries.
    pub fn edit_vec<T: Serialize + DeserializeOwned + Default>(&mut self, title: &str, list: &[T], tmpls: &Tmpls) -> io::Result<Option<Vec<T>>> {
        let mut v = serde_json::to_value(list).map_err(io::Error::other)?;
        let tmpl = serde_json::to_value(T::default()).ok();
        if !self.edit_value(title, &mut v, tmpl, tmpls)? {
            return Ok(None);
        }
        match serde_json::from_value(v) {
            Ok(t) => Ok(Some(t)),
            Err(e) => {
                self.sprint(&format!("|12Not saved: {e}"))?;
                Ok(None)
            }
        }
    }

    pub fn edit_value(&mut self, title: &str, v: &mut Value, tmpl: Option<Value>, tmpls: &Tmpls) -> io::Result<bool> {
        match v {
            Value::Object(_) => self.edit_object(title, v, tmpls),
            Value::Array(a) if tmpl.is_some() || a.iter().any(|x| x.is_object()) => self.edit_list(title, v, tmpl, tmpls),
            _ => self.edit_scalar(title, v),
        }
    }

    fn edit_object(&mut self, title: &str, v: &mut Value, tmpls: &Tmpls) -> io::Result<bool> {
        let mut changed = false;
        loop {
            let keys: Vec<String> = v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
            self.sprint(&format!("|NL|U3{title}"))?;
            // Two columns once there are too many fields for one screen.
            let two = keys.len() > 18;
            let (kw, vw) = if two { (15, 18) } else { (22, 50) };
            for (i, k) in keys.iter().enumerate() {
                let key: String = k.chars().take(kw).collect();
                self.sprompt(&format!("|U2{:>2}|U1. {key:<kw$}: |U3", i + 1))?;
                let s: String = short(&v[k]).chars().take(vw).collect();
                self.out(&s)?;
                if two && i % 2 == 0 && i + 1 < keys.len() {
                    self.out(&" ".repeat(vw + 1 - s.chars().count()))?;
                } else {
                    self.nl()?;
                }
            }
            self.sprompt(&format!("|U1Edit which (|U31-{}|U1, |U3Q|U1=done): |U3", keys.len()))?;
            let a = self.input_caps(4)?;
            let Ok(n @ 1..) = a.parse::<usize>() else {
                if a.is_empty() || a == "Q" {
                    return Ok(changed);
                }
                continue;
            };
            let Some(k) = keys.get(n - 1) else { continue };
            let tmpl = tmpls.iter().find(|(n, _)| n == k).map(|(_, t)| t.clone());
            let t = format!("{title} > {k}");
            changed |= self.edit_value(&t, &mut v[k], tmpl, tmpls)?;
        }
    }

    fn edit_list(&mut self, title: &str, v: &mut Value, tmpl: Option<Value>, tmpls: &Tmpls) -> io::Result<bool> {
        let mut changed = false;
        loop {
            let Value::Array(a) = v else { return Ok(changed) };
            self.sprint(&format!("|NL|U3{title}"))?;
            if a.is_empty() {
                self.sprint("|U1(empty)")?;
            }
            for (i, x) in a.iter().enumerate() {
                self.sprompt(&format!("|U2{:>3}|U1. |U3", i + 1))?;
                self.out(&label(x))?;
                self.nl()?;
            }
            self.sprompt("|U1[|U3#|U1] edit, [|U3I|U1]nsert, [|U3D|U1]elete, [|U3M|U1]ove, [|U3Q|U1]uit: |U3")?;
            let s = self.input_caps(5)?;
            let pick = |s: &mut Session, what: &str, max: usize| -> io::Result<Option<usize>> {
                s.sprompt(&format!("|U1{what} (1-{max}): |U3"))?;
                Ok(s.input(4)?.parse::<usize>().ok().filter(|n| (1..=max).contains(n)).map(|n| n - 1))
            };
            let len = a.len();
            match s.as_str() {
                "" | "Q" => return Ok(changed),
                "I" => {
                    let Some(at) = pick(self, "Insert before", len + 1)? else { continue };
                    let new = tmpl.clone().or_else(|| a.first().map(blank)).unwrap_or(Value::Null);
                    let Value::Array(a) = v else { continue };
                    a.insert(at, new);
                    let t = format!("{title} #{}", at + 1);
                    self.edit_value(&t, &mut a[at], None, tmpls)?;
                    changed = true;
                }
                "D" => {
                    if let Some(at) = pick(self, "Delete", len)?
                        && self.pynq(&format!("|U1Delete |U3{}|U1? ", label(&a[at])), false)? {
                            let Value::Array(a) = v else { continue };
                            a.remove(at);
                            changed = true;
                        }
                }
                "M" => {
                    let Some(from) = pick(self, "Move", len)? else { continue };
                    let Some(to) = pick(self, "To position", len)? else { continue };
                    let Value::Array(a) = v else { continue };
                    let x = a.remove(from);
                    a.insert(to, x);
                    changed = true;
                }
                n => {
                    if let Ok(n @ 1..) = n.parse::<usize>()
                        && n <= len {
                            let t = format!("{title} #{n}");
                            changed |= self.edit_value(&t, &mut a[n - 1], None, tmpls)?;
                        }
                }
            }
        }
    }

    fn edit_scalar(&mut self, title: &str, v: &mut Value) -> io::Result<bool> {
        if let Value::Bool(b) = v {
            *b = !*b;
            return Ok(true);
        }
        self.sprompt(&format!("|U1{title}: |U3"))?;
        let cur = match &*v {
            Value::String(s) => s.clone(),
            x => x.to_string(),
        };
        self.out(&cur)?;
        self.sprompt("|NL|U1New value (Enter keeps, a single space clears): |U3")?;
        let raw = self.input_main(200, false, None)?;
        if raw.is_empty() {
            return Ok(false);
        }
        let new = match &*v {
            Value::String(_) => Value::String(if raw == " " { String::new() } else { raw }),
            Value::Number(_) => match raw.trim().parse::<i64>() {
                Ok(n) => n.into(),
                Err(_) => return self.sprint("|12Not a number.").map(|_| false),
            },
            _ => match serde_json::from_str(&raw) {
                Ok(j) => j,
                Err(e) => return self.sprint(&format!("|12Invalid: {e}")).map(|_| false),
            },
        };
        *v = new;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn labels_and_blanks() {
        assert_eq!(super::label(&json!({"file": "x", "name": "General"})), "General");
        assert_eq!(super::label(&json!({"keys": "M", "cmd": "-/"})), "M  -/");
        assert_eq!(super::blank(&json!({"a": "x", "n": 3, "l": [1]})), json!({"a": "", "n": 0, "l": []}));
    }
}
