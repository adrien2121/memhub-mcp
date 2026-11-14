//! MCP note tools: write, read, edit, delete, list.

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};

mod args;

use args::{EditArgs, IdArgs, ListArgs, ReadArgs, WriteArgs};

use super::server::{MemoryServer, Receipt, failure};
use crate::domain::{Cursor, EditOp, ListQuery, NewNote, NoteId, Tag};
use crate::error::Error;

/// Wraps raw tag strings from tool arguments into domain [`Tag`]s, unchanged and in order.
///
/// # Arguments
/// * `raw` - Tag strings as sent by the client.
///
/// # Returns
/// One [`Tag`] per input string.
fn tags(raw: Vec<String>) -> Vec<Tag> {
    raw.into_iter().map(Tag).collect()
}

#[tool_router(router = notes_router, vis = "pub(crate)")]
impl MemoryServer {
    /// `write_note` tool: creates a note, or replaces the note with the same title when
    /// `overwrite` is true, on a blocking worker.
    ///
    /// # Arguments
    /// * `a` - Title, Markdown body, tags and the overwrite flag.
    ///
    /// # Returns
    /// A JSON [`Receipt`] (`id`, `version`, `updated_at`), or an `isError` result with the note
    /// service's error message (e.g. the title is taken without `overwrite`, invalid input).
    #[tool(
        description = "Create a note. Returns {id, version, updated_at}. Fails with the existing id if the title is taken, unless overwrite is true."
    )]
    async fn write_note(&self, Parameters(a): Parameters<WriteArgs>) -> CallToolResult {
        let notes = self.notes.clone();
        let note = NewNote { title: a.title, body: a.body, tags: tags(a.tags) };
        self.call(move || notes.write(note, a.overwrite).map(Receipt::from)).await
    }

    /// `read_note` tool: fetches one note by id or by title (case-insensitive).
    ///
    /// # Arguments
    /// * `a` - Exactly one of `id` or `title`.
    ///
    /// # Returns
    /// The full note as JSON, or an `isError` result if both or neither selector is given,
    /// the note does not exist, or the read fails.
    #[tool(description = "Read a note by id or by title. Returns body, tags and version.")]
    async fn read_note(&self, Parameters(a): Parameters<ReadArgs>) -> CallToolResult {
        let notes = self.notes.clone();
        match (a.id, a.title) {
            (Some(id), None) => self.call(move || notes.read_id(NoteId(id))).await,
            (None, Some(title)) => self.call(move || notes.read_title(&title)).await,
            _ => failure(Error::InvalidInput("give exactly one of id or title".into()).to_string()),
        }
    }

    /// `edit_note` tool: applies one edit operation to a note, guarded by optimistic versioning.
    ///
    /// # Arguments
    /// * `a` - Note id, the version the client last read, and exactly one operation
    ///   (`replace_body`, `append`, `replace_section` or `set_tags`).
    ///
    /// # Returns
    /// A JSON [`Receipt`] for the new version, or an `isError` result if not exactly one
    /// operation is given, the version does not match, the note or heading is missing, or the
    /// edit is invalid.
    #[tool(
        description = "Edit a note at expected_version with exactly one of replace_body, append, replace_section or set_tags. Returns {id, version, updated_at}."
    )]
    async fn edit_note(&self, Parameters(a): Parameters<EditArgs>) -> CallToolResult {
        let op = match (a.replace_body, a.append, a.replace_section, a.set_tags) {
            (Some(body), None, None, None) => EditOp::ReplaceBody(body),
            (None, Some(text), None, None) => EditOp::Append(text),
            (None, None, Some(s), None) => EditOp::ReplaceSection { heading: s.heading, content: s.content },
            (None, None, None, Some(t)) => EditOp::SetTags(tags(t)),
            _ => {
                let msg = "give exactly one of replace_body, append, replace_section or set_tags";
                return failure(Error::InvalidInput(msg.into()).to_string());
            }
        };
        let notes = self.notes.clone();
        self.call(move || notes.edit(NoteId(a.id), a.expected_version, op).map(Receipt::from)).await
    }

    /// `delete_note` tool: permanently deletes a note and purges its text.
    ///
    /// # Arguments
    /// * `a` - Id of the note to delete.
    ///
    /// # Returns
    /// The JSON string `"deleted"`, or an `isError` result if the note does not exist or the
    /// delete fails.
    #[tool(description = "Delete a note permanently; its text is purged immediately.")]
    async fn delete_note(&self, Parameters(a): Parameters<IdArgs>) -> CallToolResult {
        let notes = self.notes.clone();
        self.call(move || notes.delete(NoteId(a.id)).map(|()| "deleted")).await
    }

    /// `list_notes` tool: returns one page of notes filtered by tag and update time.
    ///
    /// # Arguments
    /// * `a` - Optional tag, update-time bounds (Unix seconds), page size (default 20, capped at
    ///   100 by the service) and the previous page's cursor.
    ///
    /// # Returns
    /// The page as JSON (notes plus `next_cursor` when more remain), or an `isError` result
    /// for an invalid cursor or a failed query.
    #[tool(description = "List notes filtered by tag and update time, one page at a time (limit 1 to 100).")]
    async fn list_notes(&self, Parameters(a): Parameters<ListArgs>) -> CallToolResult {
        let notes = self.notes.clone();
        let query = ListQuery {
            tag: a.tag.map(Tag),
            updated_after: a.updated_after,
            updated_before: a.updated_before,
            limit: a.limit.unwrap_or(20),
            cursor: a.cursor.map(Cursor),
        };
        self.call(move || notes.list(query)).await
    }
}
