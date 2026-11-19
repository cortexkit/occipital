//! A JSON value keeps only the last occurrence of a key. Check the fixture
//! before converting it to values. Unlike numbers in a canonical template,
//! numbers in tool schemas and sample calls need not be integers.

use serde::de::{DeserializeSeed, Deserializer, Error, MapAccess, SeqAccess, Visitor};
use std::{collections::HashSet, fmt};

pub(super) fn check(text: &str) -> serde_json::Result<()> {
    let mut parser = serde_json::Deserializer::from_str(text);
    Members(&mut String::new()).deserialize(&mut parser)?;
    parser.end()
}

struct Members<'a>(&'a mut String);

impl<'de> DeserializeSeed<'de> for Members<'_> {
    type Value = ();
    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Members<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON value")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        let mut seen = HashSet::new();
        let parent = self.0.len();
        while let Some(key) = map.next_key::<String>()? {
            self.0.push('/');
            self.0.push_str(&key.replace('~', "~0").replace('/', "~1"));
            if !seen.insert(key) {
                return Err(M::Error::custom(format!("duplicate_key at {}", self.0)));
            }
            map.next_value_seed(Members(self.0))?;
            self.0.truncate(parent);
        }
        Ok(())
    }

    fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<(), S::Error> {
        let parent = self.0.len();
        for index in 0usize.. {
            self.0.push('/');
            self.0.push_str(&index.to_string());
            let value = seq.next_element_seed(Members(self.0))?;
            self.0.truncate(parent);
            if value.is_none() {
                break;
            }
        }
        Ok(())
    }

    fn visit_bool<E: Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }
    fn visit_unit<E: Error>(self) -> Result<(), E> {
        Ok(())
    }
}
