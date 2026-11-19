//! Numbered transcript ranges and the rows served from them.
use super::*;

impl BrocaSource {
    /// Project entries read from the history `head` names, which the caller has
    /// checked page by page, once a second head read shows that history unchanged.
    pub(super) async fn project_window(
        &self,
        head: Head,
        served: usize,
        mut entries: Vec<Value>,
        pruned_reasoning: Vec<PrunedReasoning>,
        older: bool,
        newer: bool,
    ) -> Result<Snapshot, ReadError> {
        let after = self.read_head().await?;
        if after.history_version != head.history_version || after.incarnation != head.incarnation {
            return Err(unstable());
        }
        let mut rows = crate::app_reader::observe::synchronous("projection", || {
            self.rows_in_window(&entries, served, false, &pruned_reasoning)
        })?;
        entries.truncate(served);
        self.size_images(&mut rows).await;
        self.settle_say(&mut rows).await?;
        // The apps draw every system row as a prompt card that opens on tap, so
        // one with nothing in it, such as the entry Pi writes when a session
        // starts, is left out of pages. So is the user entry a runner wrote for a
        // send it delivered with a tool result: the model read that text inside
        // the tool's result, never as a message, and the apps would draw it as
        // the person's own. The rows kept keep their ordinals, and whether the
        // page reached the start comes from the runner's page.
        let rides = tool_result_rides(&entries);
        rows.retain(|row| {
            !is_empty_system_row(&row.display)
                && !(rides.contains(&row.mid) && row.display["role"] == "user")
        });
        let mut seen = HashSet::new();
        let mut last = None;
        let mut numbered = false;
        for row in &rows {
            if !seen.insert(row.mid.clone())
                || (row.ordinal.is_none() && numbered)
                || row
                    .ordinal
                    .is_some_and(|ordinal| last.is_some_and(|previous| ordinal <= previous))
            {
                return Err(invalid(
                    "Broca session.read returned overlapping or unordered identities",
                ));
            }
            if let Some(ordinal) = row.ordinal {
                last = Some(ordinal);
                numbered = true;
            }
        }
        self.mids.guard().extend(seen);
        let resolved = self.resolved.guard();
        for row in &mut rows {
            row.display["network"] = json!(resolved.network_at(row.written_at));
        }
        Ok(Snapshot {
            rows,
            head,
            entries,
            pruned_reasoning,
            older,
            newer,
        })
    }

    pub(super) async fn numbered_window(
        &self,
        head: Head,
        from: u64,
        end: u64,
    ) -> Result<Snapshot, ReadError> {
        let mut next = from;
        let mut entries = Vec::new();
        let mut pruned_reasoning = Vec::new();
        let older = from > FIRST_ORDINAL;
        // A runner declaring `rich_ui_stamp` adds each stamped assistant entry's stamp
        // to the same read when asked, and one declaring `read_display_facts` adds
        // each entry's client message id and its calls' times. Any other runner
        // refuses either field, so each is asked for only where declared. One
        // declaring `media_refs` serves each stored photo or file as a reference
        // instead of a text marker, when asked.
        let describe = self.describe().await;
        let stamped = describe
            .as_ref()
            .is_some_and(|describe| describe.declares("rich_ui_stamp"));
        let display_facts = describe
            .as_ref()
            .is_some_and(|describe| describe.declares("read_display_facts"));
        let (media, tool_media) = stored_media(describe.as_deref());
        // Results immediately after the page can complete calls on its last row.
        let read_end = if from < end {
            end.saturating_add(8)
                .min(head.last_ordinal.unwrap_or(0).saturating_add(1))
        } else {
            end
        };
        while next < read_end {
            let request = ReadRequest::range(next)
                .with_limit((read_end - next).min(u64::from(SOURCE_PAGE_LIMIT)))
                .with_max_bytes(2_097_152)
                .with_include_rich_ui(stamped)
                .with_include_display_facts(display_facts);
            let request = match media {
                Some(media) => request.with_media(media),
                None => request,
            };
            let request = match tool_media {
                Some(tool_media) => request.with_tool_media(tool_media),
                None => request,
            };
            let value = self.remote_read(ReadOp::Read, json_of(&request)?).await?;
            let mut page: RawPage =
                serde_json::from_value(value).map_err(|error| invalid(error.to_string()))?;
            if !page.agrees_with(&head) {
                return Err(unstable());
            }
            // The requested ordinal window, not total transcript length, bounds the work.
            if page.next_from_ordinal.is_some_and(|cursor| cursor <= next) {
                return Err(invalid("Broca session.read repeated its cursor"));
            }
            page.messages.retain(|entry| {
                entry["ordinal"]
                    .as_u64()
                    .is_some_and(|ordinal| ordinal >= next && ordinal >= from && ordinal < read_end)
            });
            // With display facts the read already carries the client ids, so no
            // display-history read is needed.
            if !display_facts {
                let served = page
                    .messages
                    .partition_point(|entry| entry["ordinal"].as_u64().is_some_and(|at| at < end));
                self.echo_client_message_ids(&mut page.messages[..served])
                    .await;
            }
            entries.extend(page.messages);
            pruned_reasoning.extend(page.pruned_reasoning);
            let Some(cursor) = page.next_from_ordinal else {
                break;
            };
            next = cursor;
        }
        let newer = head.last_ordinal.is_some_and(|last| end <= last);
        let served =
            entries.partition_point(|entry| entry["ordinal"].as_u64().is_some_and(|at| at < end));
        self.project_window(head, served, entries, pruned_reasoning, older, newer)
            .await
    }
}
