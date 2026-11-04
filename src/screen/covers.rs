//! Per-screen cover deduplication.
//!
//! Every list screen used to spawn one fetch per row. This helper keys fetches
//! by [`CoverId`] instead: distinct covers fetch once each, repeated covers
//! share one result, and entries that scroll out are dropped.

use std::collections::HashMap;

use bytes::Bytes;

use crate::library::{CoverId, ManagerHandle};
use crate::prelude::*;

#[derive(Debug, Clone)]
pub enum CoverState {
    Loading,
    Loaded(image::Handle),
    Failed,
}

#[derive(Debug, Default)]
pub struct CoverMap {
    states: HashMap<CoverId, CoverState>,
}

/// One finished cover fetch, keyed by the id the caller asked for.
pub type CoverResult = (CoverId, Result<Bytes, ()>);

impl CoverMap {
    /// Keep only wanted ids, start one fetch per missing id, and return the
    /// batch. Existing Loading/Loaded/Failed entries are left alone.
    pub fn sync(
        &mut self,
        wanted: impl IntoIterator<Item = CoverId>,
        px: u32,
        handle: &ManagerHandle,
    ) -> Task<CoverResult> {
        let wanted: Vec<CoverId> = wanted.into_iter().collect();
        self.states
            .retain(|id, _| wanted.iter().any(|wanted| wanted == id));

        let mut tasks = Vec::new();
        for id in wanted {
            if self.states.contains_key(&id) {
                continue;
            }
            self.states.insert(id.clone(), CoverState::Loading);
            let handle = handle.clone();
            let fetch = id.clone();
            tasks.push(Task::perform(
                async move { handle.cover(&fetch, px).await },
                move |result| (id, result.map_err(|_| ())),
            ));
        }

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    pub fn apply(&mut self, id: CoverId, result: Result<Bytes, ()>) {
        let Some(state) = self.states.get_mut(&id) else {
            return;
        };
        *state = match result {
            Ok(bytes) => CoverState::Loaded(image::Handle::from_bytes(bytes)),
            Err(_) => CoverState::Failed,
        };
    }

    pub fn get(&self, id: &CoverId) -> Option<&CoverState> {
        self.states.get(id)
    }

    pub fn clear(&mut self) {
        self.states.clear();
    }

    #[cfg(test)]
    pub(crate) fn tracked(&self) -> usize {
        self.states.len()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::library::{LibraryConfig, Manager, ScanMode, scan};

    use super::*;

    #[tokio::test]
    async fn deduplicates_repeated_cover_ids() {
        let temp = tempfile::TempDir::new().expect("temp");
        let data = temp.path().join("data");
        let cache = temp.path().join("cache");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/music/folders");
        let configs = vec![LibraryConfig::Offline {
            name: "Folders".into(),
            id: "folders".into(),
            root,
            scan: ScanMode::Folders,
            enabled: true,
            never_split: Vec::new(),
        }];
        let manager = Manager::open_at(&configs, &data, &cache)
            .await
            .expect("manager");
        for plan in manager.scan_plan() {
            let outcome = scan::library(plan, true).await;
            outcome.result.expect("scan");
        }

        let covers: Vec<CoverId> = manager
            .albums()
            .await
            .expect("albums")
            .into_iter()
            .filter_map(|album| album.cover)
            .collect();
        assert!(covers.len() >= 2);
        let (first, second) = (covers[0].clone(), covers[1].clone());

        let mut map = CoverMap::default();
        let _ = map.sync(
            vec![first.clone(), first.clone(), second.clone()],
            96,
            &manager.handle(),
        );
        assert_eq!(map.tracked(), 2);

        let _ = map.sync(vec![first.clone()], 96, &manager.handle());
        assert_eq!(map.tracked(), 1);
        assert!(matches!(map.get(&first), Some(CoverState::Loading)));

        map.apply(
            first.clone(),
            Ok(bytes::Bytes::from_static(b"not-an-image")),
        );
        assert!(matches!(map.get(&first), Some(CoverState::Loaded(_))));
    }
}
