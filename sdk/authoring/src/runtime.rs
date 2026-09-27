//! Preview-only value storage. This file is also embedded in private builds.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MAX_SLOTS: usize = 16_384;
const MAX_STRINGS: usize = 8 * 1024 * 1024;
const MAX_VALUES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    pub id: usize,
    pub kind: String,
    pub value: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Update {
    pub file: String,
    pub schema: String,
    pub revision: u64,
    pub values: Vec<Value>,
}
struct Slot {
    validate: fn(&str) -> bool,
    kind: &'static str,
}
#[derive(Default)]
struct File {
    schema: String,
    revision: u64,
    slots: BTreeMap<usize, Slot>,
    values: BTreeMap<usize, String>,
}
#[derive(Default)]
pub struct Store {
    files: BTreeMap<String, File>,
    strings: BTreeMap<String, &'static str>,
    string_bytes: usize,
    slots: usize,
    value_bytes: usize,
}
pub trait Literal: Copy {
    const KIND: &'static str;
    fn validate(text: &str) -> bool;
    fn decode(text: &str, strings: &BTreeMap<String, &'static str>) -> Option<Self>;
}
macro_rules! numeric {
    ($kind:literal; $($ty:ty),*) => {$ (
        impl Literal for $ty {
            const KIND: &'static str = $kind;
            fn validate(text: &str) -> bool { text.parse::<Self>().is_ok() }
            fn decode(text: &str, _: &BTreeMap<String, &'static str>) -> Option<Self> { text.parse().ok() }
        }
    )*};
}
numeric!("int"; i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);
numeric!("bool"; bool);
numeric!("char"; char);
macro_rules! floating {
    ($($ty:ty),*) => {$ (
        impl Literal for $ty {
            const KIND: &'static str = "float";
            fn validate(text: &str) -> bool { text.parse::<Self>().is_ok_and(|v| v.is_finite()) }
            fn decode(text: &str, _: &BTreeMap<String, &'static str>) -> Option<Self> { text.parse::<Self>().ok().filter(|v| v.is_finite()) }
        }
    )*};
}
floating!(f32, f64);
impl Literal for &'static str {
    const KIND: &'static str = "string";
    fn validate(text: &str) -> bool {
        text.len() <= 16 * 1024
    }
    fn decode(text: &str, strings: &BTreeMap<String, &'static str>) -> Option<Self> {
        strings.get(text).copied()
    }
}
impl Store {
    pub fn value<T: Literal>(&mut self, file: &str, schema: &str, id: usize, default: T) -> T {
        if !self.files.contains_key(file) && self.files.len() >= 1024 {
            return default;
        }
        let entry = self.files.entry(file.into()).or_default();
        if entry.schema != schema {
            self.slots = self.slots.saturating_sub(entry.slots.len());
            self.value_bytes = self
                .value_bytes
                .saturating_sub(entry.values.values().map(String::len).sum::<usize>());
            *entry = File {
                schema: schema.into(),
                ..File::default()
            };
        }
        if let std::collections::btree_map::Entry::Vacant(e) = entry.slots.entry(id) {
            if self.slots >= MAX_SLOTS {
                return default;
            }
            e.insert(Slot {
                kind: T::KIND,
                validate: T::validate,
            });
            self.slots += 1;
        }
        entry
            .values
            .get(&id)
            .and_then(|v| T::decode(v, &self.strings))
            .unwrap_or(default)
    }

    /// Validate the whole update before publishing any value. Stale source
    /// layouts, out-of-range numbers and unbounded input cannot partly apply.
    pub fn apply(&mut self, update: &Update) -> Result<bool, String> {
        if update.values.len() > 4096 {
            return Err("Too many live values".into());
        }
        let batch_bytes: usize = update.values.iter().map(|v| v.value.len()).sum();
        if batch_bytes > 1024 * 1024 {
            return Err("Live value batch exceeds 1 MiB".into());
        }
        let file = self
            .files
            .get(&update.file)
            .ok_or("This file has not composed in this preview")?;
        if file.schema != update.schema {
            return Err("Source structure changed; rebuild the preview".into());
        }
        if update.revision <= file.revision {
            return Ok(false);
        }
        let old_bytes: usize = file.values.values().map(String::len).sum();
        let retained = self
            .value_bytes
            .saturating_sub(old_bytes)
            .saturating_add(batch_bytes);
        if retained > MAX_VALUES {
            return Err("Live value budget reached; restart the preview".into());
        }
        let mut values = BTreeMap::new();
        let mut new_strings = BTreeMap::new();
        for value in &update.values {
            if value.id >= 4096
                || value.value.len() > 16 * 1024
                || values.insert(value.id, value.value.clone()).is_some()
            {
                return Err("Invalid live value batch".into());
            }
            if let Some(slot) = file.slots.get(&value.id)
                && (value.kind != slot.kind || !(slot.validate)(&value.value))
            {
                return Err(format!(
                    "Value {} does not fit its compiled {} type",
                    value.id, slot.kind
                ));
            }
            if value.kind == "string" && !self.strings.contains_key(&value.value) {
                new_strings.insert(value.value.clone(), ());
            }
        }
        let additional: usize = new_strings.keys().map(|s| s.len() + 1).sum();
        if self.string_bytes.saturating_add(additional) > MAX_STRINGS {
            return Err("Live string budget reached; restart the preview".into());
        }
        // Rust literals can be retained as &'static str by application code.
        // Interned replacements therefore live until process exit, under a
        // strict 8 MiB budget; repeated values reuse the same allocation.
        for text in new_strings.into_keys() {
            self.string_bytes += text.len() + 1;
            let retained: &'static str = Box::leak(text.clone().into_boxed_str());
            self.strings.insert(text, retained);
        }
        let file = self.files.get_mut(&update.file).expect("validated file");
        file.revision = update.revision;
        let changed = file.values != values;
        file.values = values;
        self.value_bytes = retained;
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn update(revision: u64, a: &str, b: &str) -> Update {
        Update {
            file: "src/main.rs".into(),
            schema: "a".into(),
            revision,
            values: vec![
                Value {
                    id: 0,
                    kind: "int".into(),
                    value: a.into(),
                },
                Value {
                    id: 1,
                    kind: "string".into(),
                    value: b.into(),
                },
            ],
        }
    }
    #[test]
    fn atomic_typed_updates_preserve_static_strings_and_reject_stale_edits() {
        let mut store = Store::default();
        assert_eq!(store.value("src/main.rs", "a", 0, 2u8), 2);
        assert_eq!(store.value("src/main.rs", "a", 1, "before"), "before");
        assert!(store.apply(&update(1, "7", "after")).expect("apply"));
        let held = store.value("src/main.rs", "a", 1, "before");
        assert!(store.apply(&update(2, "256", "invalid")).is_err());
        assert_eq!(store.value("src/main.rs", "a", 0, 2u8), 7);
        assert_eq!(store.value("src/main.rs", "a", 1, "before"), "after");
        assert!(!store.apply(&update(1, "8", "stale")).expect("old"));
        assert!(store.apply(&update(3, "8", "new")).expect("new"));
        assert_eq!(held, "after");
        let mut changed = update(4, "9", "wrong");
        changed.schema = "b".into();
        assert!(store.apply(&changed).is_err());
        assert_eq!(store.value("src/main.rs", "b", 0, 4u8), 4);
    }
    #[test]
    fn rejects_nonfinite_and_bounds_interned_string_storage() {
        assert!(!f32::validate("NaN"));
        assert!(!f32::validate("1e999"));
        let mut store = Store::default();
        store.value("src/main.rs", "a", 0, 0u8);
        store.value("src/main.rs", "a", 1, "");
        assert!(
            store
                .apply(&update(1, "1", &"x".repeat(16 * 1024 + 1)))
                .is_err()
        );
        store.string_bytes = MAX_STRINGS;
        assert!(store.apply(&update(2, "1", "new")).is_err());
        assert_eq!(store.value("src/main.rs", "a", 0, 0u8), 0);
    }
    #[test]
    fn bounds_retained_values_and_reclaims_recompiled_file_budget() {
        let mut store = Store::default();
        // Distinct files share interned strings but retain separate slot values.
        let values: Vec<_> = (0..64)
            .map(|id| Value {
                id,
                kind: "string".into(),
                value: "x".repeat(16 * 1024),
            })
            .collect();
        for file in 0..8 {
            let file = file.to_string();
            store.value(&file, "a", 0, "");
            assert!(
                store
                    .apply(&Update {
                        file,
                        schema: "a".into(),
                        revision: 1,
                        values: values.clone()
                    })
                    .expect("within retained byte budget")
            );
        }
        assert_eq!(store.value_bytes, MAX_VALUES);
        store.value("ninth", "a", 0, "");
        let update = Update {
            file: "ninth".into(),
            schema: "a".into(),
            revision: 1,
            values,
        };
        assert!(store.apply(&update).is_err());
        store.value("0", "b", 0, "");
        assert!(store.apply(&update).expect("reclaimed byte budget"));
        assert_eq!(store.value_bytes, MAX_VALUES);
        assert_eq!(store.string_bytes, 16 * 1024 + 1);
    }
}
