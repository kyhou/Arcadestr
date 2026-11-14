//! Browser transport/storage adapter. Event validation and ordering live in core.
#![cfg(all(target_arch = "wasm32", feature = "web"))]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use arcadestr_core::profile_metadata::{metadata_event_is_newer, validate_metadata_event};
use nostr::{Event, Kind, PublicKey};
use wasm_bindgen::{closure::Closure, JsCast};
use web_sys::{MessageEvent, WebSocket};

use crate::models::UserProfile;
use crate::store::ProfileStore;

const PREFIX: &str = "arcadestr.web.profile.";
const BOOTSTRAP_RELAYS: &[&str] = arcadestr_core::relay_defaults::DISCOVERY_RELAYS;

fn storage() -> Result<web_sys::Storage, String> {
    web_sys::window()
        .ok_or("Window unavailable")?
        .local_storage()
        .map_err(|_| "Local storage unavailable")?
        .ok_or_else(|| "Local storage unavailable".into())
}

fn profile(event: &Event, npub: &str) -> Result<UserProfile, String> {
    let author = PublicKey::parse(npub).map_err(|error| error.to_string())?;
    let content = validate_metadata_event(event, author)?;
    Ok(UserProfile {
        npub: npub.into(),
        name: content.name,
        display_name: content.display_name,
        about: content.about,
        picture: content.picture,
        website: content.website,
        nip05: content.nip05,
        lud16: content.lud16,
        nip05_verified: false,
    })
}

fn saved_event(npub: &str) -> Option<Event> {
    let raw = storage()
        .ok()?
        .get_item(&format!("{PREFIX}{npub}"))
        .ok()??;
    let event: Event = serde_json::from_str(&raw).ok()?;
    profile(&event, npub).ok()?;
    Some(event)
}

pub fn cached(npub: &str) -> Option<UserProfile> {
    profile(&saved_event(npub)?, npub).ok()
}

pub fn cached_profiles() -> Vec<UserProfile> {
    let Ok(storage) = storage() else {
        return Vec::new();
    };
    (0..storage.length().unwrap_or(0))
        .filter_map(|index| {
            let key = storage.key(index).ok()??;
            cached(key.strip_prefix(PREFIX)?)
        })
        .collect()
}

struct RelayQuery {
    socket: WebSocket,
    _open: Closure<dyn FnMut(web_sys::Event)>,
    _message: Closure<dyn FnMut(MessageEvent)>,
}

impl Drop for RelayQuery {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        let _ = self.socket.close();
    }
}

struct Lookup {
    best: Option<Event>,
    outboxes: Vec<String>,
    relay_list: Option<Event>,
    error: Option<String>,
    finished: bool,
    stores: Vec<ProfileStore>,
}

thread_local! {
    static IN_FLIGHT: RefCell<HashMap<String, Weak<RefCell<Lookup>>>> = RefCell::new(HashMap::new());
}

fn public_relay(url: &str) -> bool {
    let Ok(url) = url::Url::parse(url) else {
        return false;
    };
    url.scheme() == "wss"
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| {
            host.contains('.')
                && !host.ends_with(".local")
                && host.parse::<std::net::IpAddr>().is_err()
        })
}

fn query(
    relay: &str,
    npub: String,
    author: PublicKey,
    lookup: Rc<RefCell<Lookup>>,
) -> Result<RelayQuery, String> {
    let socket = WebSocket::new(relay).map_err(|_| "Could not open profile relay")?;
    let send_socket = socket.clone();
    let subscription = uuid::Uuid::new_v4().to_string();
    let request = serde_json::json!(["REQ", subscription, {"authors":[author.to_hex()], "kinds":[0,10002], "limit":2}]).to_string();
    let open = Closure::new(move |_: web_sys::Event| {
        let _ = send_socket.send_with_str(&request);
    });
    socket.set_onopen(Some(open.as_ref().unchecked_ref()));
    let message = Closure::new(move |message: MessageEvent| {
        let Some(raw) = message.data().as_string() else {
            return;
        };
        // Bound untrusted frames before decoding them.
        if raw.len() > 262_144 {
            return;
        }
        let Ok(frame) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return;
        };
        if frame[0] != "EVENT" || frame[1].as_str() != Some(subscription.as_str()) {
            return;
        }
        let Ok(event) = serde_json::from_value::<Event>(frame[2].clone()) else {
            return;
        };
        if event.pubkey != author || event.verify().is_err() {
            return;
        }
        let mut state = lookup.borrow_mut();
        if event.kind == Kind::RelayList {
            if state
                .relay_list
                .as_ref()
                .is_some_and(|current| !metadata_event_is_newer(&event, current))
            {
                return;
            }
            state.outboxes = event
                .tags
                .iter()
                .filter_map(|tag| {
                    let fields = tag.as_slice();
                    if fields.first().map(String::as_str) != Some("r") {
                        return None;
                    }
                    if fields.get(2).is_some_and(|mode| mode != "write") {
                        return None;
                    }
                    fields.get(1).filter(|relay| public_relay(relay)).cloned()
                })
                .take(5)
                .collect();
            state.relay_list = Some(event);
            return;
        }
        let Ok(value) = profile(&event, &npub) else {
            return;
        };
        // Another lookup or browser tab may have saved a newer event meanwhile.
        if let Some(saved) = saved_event(&npub) {
            if state
                .best
                .as_ref()
                .is_none_or(|current| metadata_event_is_newer(&saved, current))
            {
                state.best = Some(saved);
            }
        }
        if state
            .best
            .as_ref()
            .is_some_and(|current| !metadata_event_is_newer(&event, current))
        {
            return;
        }
        let persisted = storage().and_then(|storage| {
            let raw = serde_json::to_string(&event).map_err(|error| error.to_string())?;
            storage
                .set_item(&format!("{PREFIX}{npub}"), &raw)
                .map_err(|_| "Could not save profile metadata".into())
        });
        if let Err(error) = persisted {
            web_sys::console::warn_1(&error.clone().into());
            state.error = Some(error);
        }
        state.best = Some(event);
        let stores = state.stores.clone();
        drop(state);
        for store in stores {
            store.put(value.clone());
        }
    });
    socket.set_onmessage(Some(message.as_ref().unchecked_ref()));
    Ok(RelayQuery {
        socket,
        _open: open,
        _message: message,
    })
}

pub async fn fetch(npub: String, store: Option<ProfileStore>) -> Result<UserProfile, String> {
    let author = PublicKey::parse(&npub).map_err(|error| error.to_string())?;
    let existing = IN_FLIGHT.with(|active| active.borrow().get(&npub).and_then(Weak::upgrade));
    if let Some(existing) = existing {
        if !existing.borrow().finished {
            if let Some(store) = store {
                existing.borrow_mut().stores.push(store);
            }
            return wait_for_profile(&existing, &npub).await;
        }
    }
    let lookup = Rc::new(RefCell::new(Lookup {
        best: saved_event(&npub),
        outboxes: Vec::new(),
        relay_list: None,
        error: None,
        finished: false,
        stores: store.into_iter().collect(),
    }));
    IN_FLIGHT.with(|active| {
        let mut active = active.borrow_mut();
        active.retain(|_, lookup| lookup.strong_count() > 0);
        active.insert(npub.clone(), Rc::downgrade(&lookup));
    });
    let background = lookup.clone();
    let query_npub = npub.clone();
    // Retain callbacks for a bounded refresh, even after the first result is shown.
    wasm_bindgen_futures::spawn_local(async move {
        let mut relays = HashSet::new();
        let mut queries = Vec::new();
        let mut pending: Vec<String> = BOOTSTRAP_RELAYS
            .iter()
            .map(|relay| (*relay).into())
            .collect();
        for _ in 0..80 {
            pending.append(&mut background.borrow_mut().outboxes);
            for relay in pending.drain(..) {
                if relays.len() >= 8 || !relays.insert(relay.clone()) {
                    continue;
                }
                if let Ok(query) = query(&relay, query_npub.clone(), author, background.clone()) {
                    queries.push(query);
                }
            }
            gloo_timers::future::TimeoutFuture::new(100).await;
        }
        // Dropping queries removes handlers and closes every socket.
        drop(queries);
        background.borrow_mut().finished = true;
    });
    wait_for_profile(&lookup, &npub).await
}

async fn wait_for_profile(lookup: &Rc<RefCell<Lookup>>, npub: &str) -> Result<UserProfile, String> {
    loop {
        {
            let state = lookup.borrow();
            if let Some(event) = &state.best {
                return profile(event, npub);
            }
            if state.finished {
                return Err(state
                    .error
                    .clone()
                    .unwrap_or_else(|| "No account metadata found on relays".into()));
            }
        }
        gloo_timers::future::TimeoutFuture::new(50).await;
    }
}
