//! The system text Occipital serves: the text that teaches a model the
//! display tags it may write in a reply, stamped with the rich UI dialect and
//! catalog that text teaches.
//!
//! A runner asks for it with the `system_text` item of a `tool.catalog`
//! request. Occipital owns the rich UI catalog, so its answer is the one that
//! carries the `rich_ui` stamp: a runner copies that stamp onto every reply
//! of a run that froze this answer, and the phone draws a stamped reply with
//! tags only when the stamp matches the catalog it embeds. Both stamp values
//! are read from the language library's version descriptor, never written
//! here, so the stamp cannot drift from the catalog the text was generated
//! from.

use cortexkit_role_tool_provider::catalog::{self, SystemTextAnswer, SystemTextItem};
use cortexkit_role_tool_provider::rich_ui::RichUiStamp;
use serde_json::{Map, Value};

/// Why a `system_text` item was not answered: the request member at fault,
/// for an `invalid_request` refusal, or an internal failure.
#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The item asks for something Occipital does not serve; `field` names
    /// the member, such as `system_text.preset`.
    Invalid {
        /// The request member at fault.
        field: &'static str,
        /// What is wrong with it.
        message: &'static str,
    },
    /// Occipital could not build its own answer.
    Internal(String),
}

/// The stamp the reply teaching text carries: the execution dialect and the
/// catalog digest replies are stamped with, which the language library keeps
/// at catalog 1.0's first digest so every app build can draw the reply.
pub fn stamp() -> Result<RichUiStamp, String> {
    let descriptor = occipital_lang::VersionDescriptor::current();
    RichUiStamp::new(
        descriptor.execution_dialect,
        occipital_lang::reply_stamp_digest(),
    )
    .map_err(|error| format!("the language's version descriptor is not a valid stamp: {error}"))
}

/// The answer to a `system_text` item. Occipital serves one text, its
/// default, so an item naming a preset or carrying params is refused rather
/// than guessed at. `composition` is the request's composition, when it
/// carried one; the text does not depend on it, but the answer names it so a
/// runner can confirm which composition the text was resolved against.
pub fn answer(
    item: &SystemTextItem,
    composition: Option<&Map<String, Value>>,
) -> Result<SystemTextAnswer, Refusal> {
    if item.preset.is_some() {
        return Err(Refusal::Invalid {
            field: "system_text.preset",
            message: "occipital serves only its default system text",
        });
    }
    if !item.params.is_empty() {
        return Err(Refusal::Invalid {
            field: "system_text.params",
            message: "occipital's system text takes no params",
        });
    }
    let text = &occipital_lang::reply_teaching().text;
    // The text is a fixed function of the catalog compiled into this build,
    // so the digest of the text itself also serves as the preflight digest:
    // it changes exactly when the text would.
    let digest = catalog::system_text_digest(text);
    let mut answer = SystemTextAnswer::new(digest.clone(), digest)
        .with_text(text.as_str())
        // List only the tools enabled in this build; the reply teaching
        // itself stays identical even when no interactive tool is available.
        .with_tool_names(crate::manifest::TOOL_NAMES.iter().copied())
        .with_rich_ui(stamp().map_err(Refusal::Internal)?);
    if let Some(composition) = composition {
        let digest = catalog::composition_digest(&Value::Object(composition.clone()))
            .map_err(|error| Refusal::Internal(error.to_string()))?;
        answer = answer.with_composition_digest(digest);
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Replies stay stamped with catalog 1.0's first digest, which every app
    /// build knows: the version descriptor lists it as an alias of the
    /// current digest, so newer builds accept it too.
    #[test]
    fn the_stamp_is_the_first_catalog_digest_every_app_knows() {
        let descriptor = occipital_lang::VersionDescriptor::current();
        let stamp = stamp().unwrap();
        assert_eq!(stamp.dialect(), descriptor.execution_dialect);
        assert_eq!(stamp.dialect(), "1.0");
        assert_eq!(
            stamp.catalog_digest(),
            "beb0904e8b402916d84d7609a8b06db30ac308f692dcc048db2dc1138cd0c986"
        );
        assert!(descriptor
            .catalog
            .aliases
            .iter()
            .any(|alias| alias == stamp.catalog_digest()));
        assert_ne!(stamp.catalog_digest(), descriptor.catalog.digest);
    }

    #[test]
    fn the_default_item_is_answered_with_the_stamped_teaching_text() {
        let answer = answer(&SystemTextItem::default(), None).unwrap();
        let text = &occipital_lang::reply_teaching().text;
        assert_eq!(answer.text.as_deref(), Some(text.as_str()));
        assert_eq!(answer.item_digest, catalog::system_text_digest(text));
        #[cfg(feature = "show-ui")]
        assert_eq!(answer.tool_names, Some(vec!["show_ui".to_owned()]));
        #[cfg(not(feature = "show-ui"))]
        assert_eq!(answer.tool_names, Some(vec![]));
        assert_eq!(answer.rich_ui, Some(stamp().unwrap()));
        assert_eq!(answer.composition_digest, None);
        // The wire form decodes through the contract's own validating type.
        let wire = serde_json::to_value(&answer).unwrap();
        assert_eq!(
            wire["rich_ui"],
            json!({
                "dialect": "1.0",
                "catalog_digest": occipital_lang::reply_stamp_digest(),
            })
        );
        let decoded: SystemTextAnswer = serde_json::from_value(wire).unwrap();
        assert_eq!(decoded, answer);
    }

    #[test]
    fn a_composition_is_named_by_its_digest() {
        let composition = Map::from_iter([("persona".to_owned(), json!("x"))]);
        let answer = answer(&SystemTextItem::default(), Some(&composition)).unwrap();
        assert_eq!(
            answer.composition_digest,
            Some(catalog::composition_digest(&Value::Object(composition)).unwrap())
        );
    }

    #[test]
    fn a_preset_or_params_is_refused_naming_the_member() {
        let preset = SystemTextItem::default().with_preset("default");
        assert!(matches!(
            answer(&preset, None),
            Err(Refusal::Invalid {
                field: "system_text.preset",
                ..
            })
        ));
        let params = SystemTextItem::new(Map::from_iter([("tone".to_owned(), json!("x"))]));
        assert!(matches!(
            answer(&params, None),
            Err(Refusal::Invalid {
                field: "system_text.params",
                ..
            })
        ));
    }
}
