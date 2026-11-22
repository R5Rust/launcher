//! Discord Rich Presence while the game runs: "Playing R5Rust", the map, "Loading into Game" / "In Game", a timer since
//! launch, the R5Rust app icon, and "Get R5Rust" / "R5Rust Discord" buttons. Talks to the Discord app over its local pipe (no login, no network of our own); when
//! Discord isn't running nothing happens and it retries quietly. Lives on its own thread so the UI never waits on it.
//! Turned on/off in Settings (discord_rpc.txt, on by default).

use std::{
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use discord_rich_presence::{
    DiscordIpc, DiscordIpcClient,
    activity::{Activity, ActivityType, Button, Timestamps},
};

/// The R5Rust Discord application (the same one as R5Rust Bot): its name is what Discord shows as the game.
const APP_ID: &str = "1573499675447664690";
const DOWNLOAD: &str = "https://github.com/R5Rust/launcher/releases/latest/download/R5Rust-Launcher.exe";
const DISCORD: &str = "https://discord.gg/kcmDzhJfk8";

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Playing {
    /// first line, e.g. "Firing Range"
    pub map: String,
    /// second line: "Loading into Game" / "In Game"
    pub state: String,
}

enum Msg {
    Show(Playing),
    Clear,
}

pub struct Presence {
    tx: Sender<Msg>,
    shown: Option<Playing>,
}

impl Presence {
    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || run(rx));
        Self { tx, shown: None }
    }

    /// Call every frame with what should show (None: nothing). Only changes are sent to the thread.
    pub fn set(&mut self, want: Option<Playing>) {
        if want == self.shown {
            return;
        }
        let _ = self.tx.send(match &want {
            Some(p) => Msg::Show(p.clone()),
            None => Msg::Clear,
        });
        self.shown = want;
    }
}

fn run(rx: Receiver<Msg>) {
    let mut client: Option<DiscordIpcClient> = None;
    let mut want: Option<(Playing, i64)> = None;
    let mut dirty = false;
    let mut next_try = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Msg::Show(p)) => {
                // the timer runs from launch: a state change (loading -> in game) keeps it
                let start = want.as_ref().map(|(_, s)| *s).unwrap_or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0));
                want = Some((p, start));
                dirty = true;
                next_try = Instant::now();
            }
            Ok(Msg::Clear) => {
                want = None;
                if let Some(mut c) = client.take() {
                    let _ = c.clear_activity();
                    let _ = c.close();
                }
                dirty = false;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if let Some(mut c) = client.take() {
                    let _ = c.clear_activity();
                    let _ = c.close();
                }
                return;
            }
        }
        let Some((p, start)) = &want else { continue };
        if !dirty || Instant::now() < next_try {
            continue;
        }
        if client.is_none() {
            let mut c = DiscordIpcClient::new(APP_ID);
            if c.connect().is_err() {
                // Discord isn't running (or not yet): try again in a bit
                next_try = Instant::now() + Duration::from_secs(20);
                continue;
            }
            client = Some(c);
        }
        let c = client.as_mut().expect("connected above");
        let activity = Activity::new()
            .activity_type(ActivityType::Playing)
            .details(p.map.as_str())
            .state(p.state.as_str())
            .timestamps(Timestamps::new().start(*start))
            .buttons(vec![Button::new("Get R5Rust", DOWNLOAD), Button::new("R5Rust Discord", DISCORD)]);
        if c.set_activity(activity).is_ok() {
            dirty = false;
        } else {
            // the pipe broke (Discord restarted): reconnect on the next round
            client = None;
            next_try = Instant::now() + Duration::from_secs(10);
        }
    }
}
