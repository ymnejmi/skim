#[macro_use]
extern crate log;
#[macro_use]
extern crate lazy_static;
mod ansi;
mod event;
mod field;
mod header;
mod input;
mod item;
mod matcher;
mod model;
mod options;
mod orderedvec;
mod output;
mod previewer;
mod query;
mod reader;
mod score;
mod selection;
mod spinlock;
mod theme;
mod util;

use crate::event::Event::*;
use crate::event::{EventReceiver, EventSender};
use crate::model::Model;
pub use crate::options::{SkimOptions, SkimOptionsBuilder};
pub use crate::output::SkimOutput;
use crate::reader::Reader;
use nix::unistd::isatty;
use std::io::BufRead;
use std::io::BufReader;
use std::os::unix::io::AsRawFd;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tuikit::term::{Term, TermHeight, TermOptions};

const REFRESH_DURATION: u64 = 25;

pub struct Skim {}

impl Skim {
    pub fn run_with(options: &SkimOptions, source: Option<Box<BufRead + Send>>) -> Option<SkimOutput> {
        let min_height = options
            .min_height
            .map(Skim::parse_height_string)
            .expect("min_height should have default values");
        let height = options
            .height
            .map(Skim::parse_height_string)
            .expect("height should have default values");

        let (tx, rx): (EventSender, EventReceiver) = channel();
        let term = Arc::new(Term::with_options(TermOptions::default().min_height(min_height).height(height)).unwrap());

        //------------------------------------------------------------------------------
        // input
        let mut input = input::Input::new();
        input.parse_keymaps(&options.bind);
        input.parse_expect_keys(options.expect.as_ref().map(|x| &**x));
        let tx_clone = tx.clone();
        let term_clone = term.clone();
        thread::spawn(move || 'outer: loop {
            if let Ok(key) = term_clone.poll_event() {
                for (ev, arg) in input.translate_event(key).into_iter() {
                    let _ = tx_clone.send((ev, arg));
                    if ev == EvActAccept || ev == EvActAbort {
                        break 'outer;
                    }
                }
            }
        });

        //------------------------------------------------------------------------------
        // reader

        // in piped situation(e.g. `echo "a" | sk`) set source to the pipe
        let source = source.or_else(|| {
            let stdin = std::io::stdin();
            if !isatty(stdin.as_raw_fd()).unwrap_or(true) {
                Some(Box::new(BufReader::new(stdin)))
            } else {
                None
            }
        });

        let reader = Reader::with_options(&options).source(source);

        //------------------------------------------------------------------------------
        // start a timer for notifying refresh
        let tx_clone = tx.clone();
        thread::spawn(move || loop {
            let _ = tx_clone.send((EvHeartBeat, Box::new(true)));
            thread::sleep(Duration::from_millis(REFRESH_DURATION));
        });

        //------------------------------------------------------------------------------
        // model + previewer
        let mut model = Model::new(rx, tx, reader, term, &options);
        model.start()
    }

    // 10 -> TermHeight::Fixed(10)
    // 10% -> TermHeight::Percent(10)
    fn parse_height_string(string: &str) -> TermHeight {
        if string.ends_with('%') {
            TermHeight::Percent(string[0..string.len() - 1].parse().unwrap_or(100))
        } else {
            TermHeight::Fixed(string.parse().unwrap_or(0))
        }
    }
}
