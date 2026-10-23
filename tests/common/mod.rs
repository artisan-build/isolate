#![allow(dead_code)]

use std::collections::HashMap;
use std::future::pending;
use std::sync::{Arc, Mutex};

use isolate::store::{ObjectStore, StoreError, StoreFuture};

#[derive(Default)]
struct State {
    objects: HashMap<String, Vec<u8>>,
    errors: HashMap<String, StoreError>,
    calls: HashMap<String, usize>,
    blocked: bool,
}

#[derive(Clone, Default)]
pub struct FakeStore {
    state: Arc<Mutex<State>>,
}

impl FakeStore {
    pub fn object(&self, key: &str, value: impl Into<Vec<u8>>) {
        let mut state = self.state.lock().expect("fake store lock");
        state.objects.insert(key.to_owned(), value.into());
        state.errors.remove(key);
    }

    pub fn remove(&self, key: &str) {
        let mut state = self.state.lock().expect("fake store lock");
        state.objects.remove(key);
        state.errors.remove(key);
    }

    pub fn error(&self, key: &str) {
        let mut state = self.state.lock().expect("fake store lock");
        state.objects.remove(key);
        state.errors.insert(key.to_owned(), StoreError::Unavailable);
    }

    pub fn block(&self) {
        self.state.lock().expect("fake store lock").blocked = true;
    }

    pub fn calls(&self, key: &str) -> usize {
        *self
            .state
            .lock()
            .expect("fake store lock")
            .calls
            .get(key)
            .unwrap_or(&0)
    }
}

impl ObjectStore for FakeStore {
    fn get<'a>(&'a self, key: &'a str) -> StoreFuture<'a> {
        Box::pin(async move {
            let (blocked, value, error) = {
                let mut state = self.state.lock().expect("fake store lock");
                *state.calls.entry(key.to_owned()).or_default() += 1;
                (
                    state.blocked,
                    state.objects.get(key).cloned(),
                    state.errors.get(key).copied(),
                )
            };
            if blocked {
                pending::<()>().await;
            }
            if let Some(error) = error {
                return Err(error);
            }
            value.ok_or(StoreError::Missing)
        })
    }
}
