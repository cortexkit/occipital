//! Kept answers shared across one-shot reads. Live sources pin their answers;
//! among closed chats keep only the 128 most recently opened.

use super::{DescribeSlot, Mutex, SessionRef};
use crate::app_reader::message_tool::MessageToolSlot;
use crate::app_reader::Guard as _;
use std::{collections::VecDeque, sync::Arc};

#[derive(Default)]
pub(super) struct Metadata(VecDeque<Entry>);

struct Entry {
    key: SessionRef,
    describe: DescribeSlot,
    tool: MessageToolSlot,
    sources: usize,
}

pub(super) struct Lease(Arc<Mutex<Metadata>>, SessionRef);

impl Metadata {
    pub(super) fn touch(&mut self, key: &SessionRef) {
        if let Some(index) = self.0.iter().position(|entry| &entry.key == key) {
            if let Some(entry) = self.0.remove(index) {
                self.0.push_back(entry);
            }
        }
    }

    pub(super) fn acquire(
        cache: &Arc<Mutex<Self>>,
        key: &SessionRef,
    ) -> (Lease, DescribeSlot, MessageToolSlot) {
        let mut state = cache.guard();
        let mut entry = state
            .0
            .iter()
            .position(|entry| &entry.key == key)
            .and_then(|index| state.0.remove(index))
            .unwrap_or_else(|| Entry {
                key: key.clone(),
                describe: Default::default(),
                tool: Default::default(),
                sources: 0,
            });
        entry.sources += 1;
        let result = (
            Lease(cache.clone(), key.clone()),
            entry.describe.clone(),
            entry.tool.clone(),
        );
        state.0.push_back(entry);
        result
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut cache = self.0.guard();
        if let Some(entry) = cache.0.iter_mut().find(|entry| entry.key == self.1) {
            entry.sources -= 1;
        }
        if cache.0.iter().filter(|entry| entry.sources == 0).count() > 128 {
            if let Some(index) = cache.0.iter().position(|entry| entry.sources == 0) {
                cache.0.remove(index);
            }
        }
    }
}
