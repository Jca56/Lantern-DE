//! Spelling: the words the dictionary doesn't know, underlined, and what
//! the right-click menu offers for one. US English loads on a thread of
//! its own at start; until it has (or when it was never fetched) nothing
//! is underlined.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use lntrn_spell::Speller;

/// The dictionary checked against.
const LANG: &str = "en_US";
/// How many suggestions the menu offers.
const SUGGESTIONS: usize = 5;
/// Paragraphs remembered checked; past this the memory starts over.
const REMEMBERED: usize = 4096;

pub struct Spelling {
    /// Loading was asked for.
    started: bool,
    speller: Option<Speller>,
    loading: Option<Receiver<Result<Speller, String>>>,
    /// Why there is no dictionary, when there isn't.
    missing: Option<String>,
    /// Each paragraph's text, hashed, to where its misspellings are.
    checked: HashMap<u64, Rc<[Range<usize>]>>,
}

impl Spelling {
    /// Nothing loaded until [`Spelling::want`] asks.
    pub fn new() -> Spelling {
        Spelling { started: false, speller: None, loading: None, missing: None, checked: HashMap::new() }
    }

    /// Start loading the dictionary, the first time spelling is wanted.
    pub fn want(&mut self) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("spelling".into()).spawn(move || {
            let speller = Speller::open(LANG).map_err(|e| e.to_string());
            if let Ok(s) = &speller {
                s.dictionary().prepare();
            }
            let _ = tx.send(speller);
        });
        match spawned {
            Ok(_) => self.loading = Some(rx),
            Err(e) => self.missing = Some(format!("no thread to load it on: {e}")),
        }
    }

    /// Whether the dictionary is still on its way.
    pub fn loading(&self) -> bool {
        self.loading.is_some()
    }

    /// Take the dictionary when it has loaded. `true` when it arrived (or
    /// turned out missing) this time.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.loading else { return false };
        let got = match rx.try_recv() {
            Ok(got) => got,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err("the loader stopped".to_owned()),
        };
        self.loading = None;
        match got {
            Ok(speller) => self.speller = Some(speller),
            Err(e) => {
                lntrn_core::log_warn!("no spelling: {e}");
                self.missing = Some(e);
            }
        }
        self.checked.clear();
        true
    }

    /// Why there is no dictionary, once that is known.
    pub fn missing(&self) -> Option<&str> {
        self.missing.as_deref()
    }

    /// Where the misspelled words of a paragraph's `text` are.
    pub fn misspelled(&mut self, text: &str) -> Rc<[Range<usize>]> {
        let Some(speller) = &self.speller else { return Rc::from([]) };
        let mut h = DefaultHasher::new();
        text.hash(&mut h);
        let key = h.finish();
        if let Some(found) = self.checked.get(&key) {
            return found.clone();
        }
        if self.checked.len() >= REMEMBERED {
            self.checked.clear();
        }
        let found: Rc<[Range<usize>]> = speller.misspelled(text).into();
        self.checked.insert(key, found.clone());
        found
    }

    /// Where the misspelled word of `text` that byte `at` is in, or at the
    /// end of, is.
    pub fn range_at(&mut self, text: &str, at: usize) -> Option<Range<usize>> {
        self.misspelled(text).iter().find(|r| r.start <= at && at <= r.end).cloned()
    }

    /// What `word` may have meant, likeliest first.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        self.speller.as_ref().map(|s| s.suggest(word, SUGGESTIONS)).unwrap_or_default()
    }

    /// Take `word` as right from now on, in every Lantern app.
    pub fn add(&mut self, word: &str) -> Result<(), String> {
        let Some(speller) = &mut self.speller else { return Ok(()) };
        self.checked.clear();
        speller.add(word).map_err(|e| e.to_string())
    }

    /// Take `word` as right until Notepad closes.
    pub fn ignore(&mut self, word: &str) {
        if let Some(speller) = &mut self.speller {
            speller.ignore(word);
            self.checked.clear();
        }
    }
}
