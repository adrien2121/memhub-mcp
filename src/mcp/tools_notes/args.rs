//! Deserialized argument shapes for the note tools; each field's doc becomes its JSON schema description.

use rmcp::schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(super) struct WriteArgs {
    /// Unique title (case-insensitive).
    pub(super) title: String,
    /// Markdown body, at most 1 MiB (1,048,576 bytes).
    pub(super) body: String,
    /// Tags.
    #[serde(default)]
    pub(super) tags: Vec<String>,
    /// Replace the note that already has this title instead of failing.
    #[serde(default)]
    pub(super) overwrite: bool,
}

#[derive(Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(super) struct ReadArgs {
    /// Note id; give either `id` or `title`.
    pub(super) id: Option<i64>,
    /// Note title; give either `id` or `title`.
    pub(super) title: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(super) struct Section {
    /// Heading text, without the leading `#`s.
    pub(super) heading: String,
    /// New content under that heading.
    pub(super) content: String,
}

#[derive(Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(super) struct EditArgs {
    /// Note id.
    pub(super) id: i64,
    /// Version read last; a mismatch is a conflict.
    pub(super) expected_version: i64,
    /// Operation: replace the whole body.
    pub(super) replace_body: Option<String>,
    /// Operation: append text to the body.
    pub(super) append: Option<String>,
    /// Operation: replace the content under a Markdown heading.
    pub(super) replace_section: Option<Section>,
    /// Operation: replace the tag set.
    pub(super) set_tags: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(super) struct IdArgs {
    /// Note id.
    pub(super) id: i64,
}

#[derive(Deserialize, JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub(super) struct ListArgs {
    /// Only notes with this tag.
    pub(super) tag: Option<String>,
    /// Only notes updated at or after this Unix time.
    pub(super) updated_after: Option<i64>,
    /// Only notes updated before this Unix time.
    pub(super) updated_before: Option<i64>,
    /// Page size, 1 to 100 (default 20; larger values are capped at 100).
    pub(super) limit: Option<usize>,
    /// `next_cursor` from the previous page.
    pub(super) cursor: Option<String>,
}
