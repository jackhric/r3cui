# `crates/kodi`: r3cui's own Kodi client (design)

Status: implemented (`crates/kodi`, on `crates/wsclient`; its driver, `src/driver/`, is behind the default `driver` feature). The text below is the original design and may differ in detail from the code. Source of truth for method, type and notification names is the
`JSONRPC.Introspect` output from the owner's Kodi 21.3 (JSON-RPC API **13.5.0**), which this
design vendors (see §1, *Schema*). Names in `code` that look like `Player.Seek` or
`Player.Audio.Stream` are exact ids from that file.

## 1. Summary and key decisions

r3cui stops using the Remote 3 core's Kodi integration and talks to Kodi itself. A new crate,
`crates/kodi`, owns all of it: setup, discovery, the connection, the state, commands and artwork.
Like `crates/coreapi` it has no UI dependencies. The UI subscribes to one
`watch::Receiver<KodiState>` and sends commands through a cloneable `KodiHandle`. The core is
used for the remote's own state (power, battery, Wi-Fi); the Home Assistant receiver is another driver (`docs/drivers.md`).

| Decision | Choice | Reason |
|---|---|---|
| Types | **Hand-written types for the subset used, checked against the vendored schema by a test** | Kodi's schema is draft-03 style (`required: true` on properties, multi-`extends`, `type: [..]` unions, ~100-field `List.Item.Base`). Generated code would be large and mostly `Option<_>`. Hand-written types give `Duration`, enums and newtypes, and the test stops them drifting from the spec. |
| Main transport | **WebSocket JSON-RPC on 9090** (`ws://host:9090/jsonrpc`) | It is the only transport that pushes notifications. One connection carries requests and notifications. |
| Second transport | **HTTP on 8080** (`/jsonrpc`, `/image/`) | Artwork only comes over HTTP. It is also a fallback when 9090 is closed, and it handles the heavy list fetches (see §2, *Transport*). |
| Core shape | **Sans-IO engine**: `engine.rs` takes events (notifications, answers, key presses, ticks) and returns state changes plus effects (queries to send, timers) | All of the logic (timeline, focus prediction, which query each notification triggers) becomes pure and unit-testable. The IO loop stays thin. |
| State | One `KodiState`, published through `tokio::sync::watch`, with per-part revision counters and the list behind an `Arc` | One subscription for the UI. Cloning is cheap and diffing is easy. |
| Ordering | One writer task per connection, FIFO, with requests matched by `id` | Send order is call order, so the old `PRESS_GAP` reorder queue goes away. The reordering happened in the core's integration, not in Kodi. |
| Crates | `tokio-tungstenite` (already in the workspace, no TLS), `serde`/`serde_json` (with `raw_value`), `thiserror`, `ureq` 2 (already linked, no TLS) behind `spawn_blocking` for HTTP, `mdns-sd` behind a `discovery` feature | No new TLS stack and no hyper/reqwest. All of it is pure Rust and cross-compiles for aarch64 without trouble. |
| Fake source | `FakeKodi` sits behind the same frame interface as a real connection | Demo, preview and tests run the real engine. The UI has no `if demo` branch for Kodi. |
| Schema | Vendor at `crates/kodi/schema/kodi-21.3-jsonrpc-13.5.json` | Used by the conformance test and the fixture tests. Never read at runtime or build time. |

**Schema vendoring.** Commit the introspect output, pretty-printed (~270 KB), as
`crates/kodi/schema/kodi-21.3-jsonrpc-13.5.json`. Re-fetch it with
`{"method":"JSONRPC.Introspect","params":{"getdescriptions":true,"getmetadata":true,"filterbytransport":false}}`.
`filterbytransport` defaults to `true`, and over HTTP that would drop what HTTP cannot do. Kodi's own
`xbmc/interfaces/json-rpc/schema/{methods,types,notifications}.json` are the inputs from which this
file is built. Vendoring the introspect output is better because it is exactly what the owner's Kodi
serves. When we support another version, add a second file next to it and run the test against both.

## 2. Module layout and public API

```
crates/kodi/
  Cargo.toml
  schema/kodi-21.3-jsonrpc-13.5.json   vendored Introspect output (tests only)
  src/
    lib.rs          re-exports; KodiService::start; crate docs pointing here
    config.rs       KodiConfig (host, ports, credentials, prefs) + Endpoint (URL building)
    error.rs        the one Error type
    ids.rs          newtypes: PlayerId, MovieId, EpisodeId, TvShowId, SeasonId, SetId, MediaId,
                    AudioStreamIndex, SubtitleIndex, WindowId, RequestId
    wire.rs         shared wire types: Time (Global.Time <-> Duration), Speed, Artwork/ArtRef,
                    Audio/Subtitle stream structs, ListEntry, Sort; serde only
    rpc.rs          Method trait, request/response envelope, RpcError, frame classification
    methods/
      player.rs     Player.* request structs
      input.rs      Input.* request structs + InputAction enum (subset of Input.Action)
      gui.rs        GUI.GetProperties, XBMC.GetInfoLabels/GetInfoBooleans + InfoLabel enum
      library.rs    Files.GetDirectory, VideoLibrary.Get*Details
      system.rs     JSONRPC.Version/Ping, Application.GetProperties, Settings.GetSettingValue
    notification.rs Notification enum + parse(method, params)
    state.rs        KodiState and its parts (plain data, no logic beyond accessors)
    engine.rs       sans-IO brain: apply(Event) -> Vec<Effect>; owns prediction + poll schedule
    browse.rs       pure: InfoLabel answers -> Browse; GUI sort label -> List.Sort; grid model
    labels.rs       pure: language names, subtitle/audio labels, AudioCodec parsing
    art.rs          image:// encode/decode (the one encoder), resize rules, ArtFetcher + cache
    transport/
      mod.rs        Frames (in/out channel pair) + connect(): picks ws or http
      ws.rs         WebSocket connection: reader, writer, ping
      http.rs       HTTP JSON-RPC + /image/ via ureq in spawn_blocking; basic auth
    session.rs      IO loop: runs engine, matches ids, timeouts, reconnect/backoff
    handle.rs       KodiHandle: the typed command API the UI calls
    probe.rs        connection test with readable diagnoses
    discover.rs     mDNS browse for Kodi services (feature "discovery")
    fake.rs         FakeKodi: an in-process Kodi (player, library, GUI grid) for demo + tests
  tests/
    schema.rs       every Method/enum/field we use exists in the vendored schema
    fixtures/       recorded frames: responses/*.json, notifications/*.json, sessions/*.jsonl
    engine.rs       replay sessions/*.jsonl through the engine, assert states
  examples/
    probe.rs        `cargo run -p kodi --example probe -- kodi.lan` (prints a ProbeReport)
    record.rs       logs every frame of a live session to JSONL, for new fixtures
```

There are 25 source files. To find a feature: a command is in `handle.rs`, its wire format is in
`methods/<namespace>.rs`, its state effect is in `engine.rs`, and the field it shows is in `state.rs`.

### Config

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KodiConfig {
    pub host: String,                    // name or IP
    #[serde(default = "http_port")] pub http_port: u16,
    #[serde(default = "ws_port")]   pub ws_port: u16,
    pub credentials: Option<Credentials>,
    #[serde(default)] pub transport: TransportPref,      // Auto | WebSocket | Http
    #[serde(default)] pub subtitle_reload: SubtitleReload,
    pub instance: Option<String>,        // mDNS uuid TXT, to re-find Kodi after an IP change
}
pub struct Credentials { pub username: String, pub password: String }
pub enum TransportPref { Auto, WebSocket, Http }
pub enum SubtitleReload { Off, InPlace, Back(Duration) }   // default InPlace

pub struct Endpoint { /* derived from KodiConfig */ }
impl Endpoint {
    pub fn ws_url(&self) -> String;
    pub fn rpc_url(&self) -> String;
    pub fn image_url(&self, art: &ArtRef) -> String;
}
```

The UI keeps this under a `"kodi"` key in its `settings.json`. `settings.rs` embeds
`Option<kodi::KodiConfig>` and gains an atomic save (write a temp file, then rename). Today it only
reads.

### Service and handle

```rust
pub enum Source { Kodi(KodiConfig), Fake(FakeKodi) }

pub struct KodiService;
impl KodiService {
    /// Spawns the session on the current tokio runtime.
    pub fn start(source: Source) -> KodiHandle;
}

#[derive(Clone)]
pub struct KodiHandle { /* mpsc::Sender<Command>, watch::Receiver<KodiState> */ }
impl KodiHandle {
    pub fn state(&self) -> watch::Receiver<KodiState>;
    pub fn reconfigure(&self, cfg: Option<KodiConfig>);   // None = forget Kodi
    pub fn set_active(&self, a: Activity);                // Awake/Asleep, WallVisible: drives polling

    // Fire-and-forget; failures land in KodiState::last_error. Never block the UI.
    pub fn nav(&self, n: Nav);                            // Up Down Left Right Select Back Home ContextMenu Info Osd
    pub fn action(&self, a: InputAction);                 // pageup, skipnext, osd, ...
    pub fn play_pause(&self);
    pub fn stop(&self);

    // Awaitable where the UI wants to know.
    pub async fn seek_to(&self, to: Duration) -> Result<Timeline>;
    pub async fn seek_by(&self, secs: i32) -> Result<Timeline>;
    pub async fn set_audio(&self, s: AudioStreamIndex) -> Result<()>;
    pub async fn set_subtitle(&self, s: SubtitleChoice) -> Result<()>;
    pub async fn open(&self, item: MediaId, from: StartAt) -> Result<()>;
    pub async fn send_text(&self, text: &str, done: bool) -> Result<()>;
    pub async fn art(&self, art: &ArtRef, size: ArtSize) -> Result<Arc<[u8]>>;
    /// Typed escape hatch for anything not wrapped yet.
    pub async fn call<M: Method>(&self, m: M) -> Result<M::Output>;
}

pub enum Nav { Up, Down, Left, Right, Select, Back, Home, ContextMenu, Info, Osd }
pub enum SubtitleChoice { Off, Track(SubtitleIndex) }
pub enum StartAt { Beginning, Resume, At(Duration) }
pub enum Activity { Asleep, Awake { wall_visible: bool } }
pub enum ArtSize { Poster /* 136x204 */, Large /* 384x576 */, Backdrop, Original }
```

### RPC trait

```rust
pub trait Method: Serialize + Send + 'static {
    const NAME: &'static str;            // e.g. "Player.GetProperties"
    type Output: DeserializeOwned + Send + 'static;
}

// methods/player.rs
#[derive(Serialize)]
pub struct GetProperties { pub playerid: PlayerId, pub properties: &'static [PlayerProperty] }
impl Method for GetProperties { const NAME: &str = "Player.GetProperties"; type Output = PlayerProps; }

#[derive(Serialize)]
pub struct Seek { pub playerid: PlayerId, pub value: SeekTo }
#[derive(Serialize)] #[serde(untagged)]
pub enum SeekTo { Time { time: Time }, Seconds { seconds: i32 } }
impl Method for Seek { const NAME: &str = "Player.Seek"; type Output = SeekResult; }
```

The envelope uses `Box<serde_json::value::RawValue>` for `result`, so a response is matched by
`id` first and only then decoded into `M::Output`. No `serde_json::Value` is public.

Closed sets are enums whose serde names come from the schema: `PlayerProperty`
(⊆ `Player.Property.Name`), `PlayerKind` (`Player.Type`), `InputAction` (⊆ the 215-value
`Input.Action`), `SortMethod` (⊆ `List.Sort.method`), `ItemKind` (`Notifications.Item.Type`),
`GuiProperty` (`GUI.Property.Name`). `Speed` maps the integers of `Player.SetSpeed`
(−32…32, powers of two) to
`enum Speed { Paused, Normal, Forward(Mult), Backward(Mult) }` with `enum Mult { X2, X4, X8, X16, X32 }`.
`Time` converts to and from `Duration` and checks the schema limits (minutes and seconds ≤ 59,
milliseconds ≤ 999). Open sets, such as `Container.Content` and audio codec ids, are enums with an
`Other(String)` variant.

### State

```rust
#[derive(Clone, Debug, Default)]
pub struct KodiState {
    pub link: Link,
    pub server: Option<ServerInfo>,          // name, version "21.3", api 13.5.0, transport in use
    pub playback: Option<Playback>,          // None = nothing playing
    pub browse: Browse,
    pub screensaver: bool,                   // GUI.OnScreensaverActivated / OnDPMSActivated
    pub input_request: Option<InputRequest>, // Input.OnInputRequested, cleared by OnInputFinished
    pub last_error: Option<(Instant, Error)>,
    pub rev: Revisions,                      // { playback, timeline, streams, browse, list } u64 each
}

pub enum Link {
    Unconfigured,
    Connecting { attempt: u32 },
    Up { transport: TransportKind, notifications: bool },
    Down { error: Error, retry_in: Duration },
}

pub struct Playback {
    pub player: PlayerId,
    pub kind: PlayerKind,
    pub item: NowPlaying,
    pub timeline: Timeline,
    pub audio: Vec<AudioStream>, pub audio_current: Option<AudioStreamIndex>,
    pub subtitles: Vec<SubtitleStream>, pub subtitle: Option<SubtitleIndex>, // None = off
    pub chapter: Option<Chapter>,             // { number, count, name }
    pub can_seek: bool, pub live: bool,
}
pub struct NowPlaying {
    pub id: Option<MediaId>,                  // MediaId::Movie(MovieId) | Episode(EpisodeId) | ...
    pub title: String,
    pub show: Option<EpisodeOf>,              // { show_title, season, episode, tvshow: Option<TvShowId> }
    pub year: Option<u16>,
    pub art: Art,                             // poster, fanart, thumb: Option<ArtRef>
    pub runtime: Option<Duration>,
}
pub struct Timeline { pub position: Duration, pub total: Duration, pub speed: Speed, pub measured_at: Instant }
impl Timeline { pub fn position_at(&self, now: Instant) -> Duration; } // speed-aware, clamped to total

pub struct AudioStream { pub index: AudioStreamIndex, pub name: String, pub language: Lang,
                         pub codec: AudioCodec, pub channels: u8, pub bitrate: u32,
                         pub default: bool, pub original: bool, pub impaired: bool }
pub struct SubtitleStream { pub index: SubtitleIndex, pub name: String, pub language: Lang,
                            pub forced: bool, pub impaired: bool, pub default: bool }
impl AudioStream { pub fn label(&self) -> String; }     // labels.rs; "English (Commentary)"
impl SubtitleStream { pub fn label(&self) -> String; }  // "French · Forced"

pub struct Browse {
    pub window: Option<Window>,               // GUI.GetProperties currentwindow → Window/WindowId
    pub container: Option<Container>,
    pub list: Option<Arc<Listing>>,           // the wall's items, in Kodi's order
    pub focus: Option<Focus>,                 // predicted or confirmed
}
pub struct Container { pub content: Content, pub path: String, pub sort: Sort, pub num_items: Option<usize> }
pub enum Content { Movies, TvShows, Seasons, Episodes, Sets, MusicVideos, Files, None, Other(String) }
pub struct Focus { pub index: usize, pub id: Option<MediaId>, pub grid: Grid, pub confirmed: bool }
pub struct Grid { pub column: Option<u16>, pub row: Option<u16>, pub columns: Option<u16> }
pub struct Listing { pub key: ListingKey, pub items: Vec<ListEntry> }   // key = (path, sort)
pub struct ListEntry { pub id: MediaId, pub title: String, pub year: Option<u16>, pub art: Art,
                       pub rating: Option<f32>, pub mpaa: String, pub genres: Vec<String>, pub plot: String,
                       pub detail: EntryDetail }  // Movie{runtime, resume, playcount} | Show{seasons, episodes, watched}
```

### Notifications and errors

```rust
pub enum Notification {
    Play(PlayerEvent), AvStart(PlayerEvent), AvChange(PlayerEvent),
    Pause(PlayerEvent), Resume(PlayerEvent), SpeedChanged(PlayerEvent),
    Seek(SeekEvent),                    // player + time + seekoffset (Player.Notifications.Player.Seek)
    Stop { item: Option<NotifyItem>, end: bool },
    PropertyChanged { player: PlayerId },
    Screensaver(bool), Dpms(bool),
    InputRequested(InputRequest), InputFinished,
    LibraryUpdated { kind: VideoKind, id: MediaId, playcount: Option<u32>, added: bool },
    LibraryRemoved { kind: VideoKind, id: MediaId },
    LibraryChanged,                     // OnScanFinished / OnCleanFinished / OnRefresh
    Quit, Restart, Sleep, Wake,
    Unknown { method: String },         // logged once per method, otherwise ignored
}
pub struct PlayerEvent { pub player: PlayerId, pub speed: Speed, pub item: NotifyItem }
pub enum NotifyItem { Library { kind: ItemKind, id: MediaId },
                      Movie { title: String, year: Option<u16> },
                      Episode { title: String, show: String, season: u32, episode: u32 },
                      Unknown }       // the Notifications.Item union

#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("Kodi is not set up")]                    NotConfigured,
    #[error("cannot reach {endpoint}: {reason}")]     Unreachable { endpoint: Arc<str>, reason: Arc<str> },
    #[error("Kodi refused the username or password")] Unauthorized,
    #[error("{method} timed out after {after:?}")]    Timeout { method: &'static str, after: Duration },
    #[error("{method}: Kodi error {code}: {message}")] Rpc { method: &'static str, code: i32, message: Arc<str> },
    #[error("{method}: unexpected reply: {detail}")]  Decode { method: &'static str, detail: Arc<str> },
    #[error("nothing is playing")]                    NothingPlaying,
    #[error("connection closed")]                     Closed,
}
pub type Result<T> = std::result::Result<T, Error>;
```

`Error` is `Clone` so it can live in `KodiState`. Connection-level errors go to `Link::Down`.
Errors from single commands go to `last_error`, which the UI may show as a toast and which never
blocks anything. `Rpc.code` keeps Kodi's codes (`-32602` invalid params, `-32100` failed to
execute, `-32601` method not found), so an older Kodi without a method shows up as a clear error.

### Transport

- **One WS connection** is the default. The writer task takes `(RequestId, &'static str, Box<RawValue>)`
  from an mpsc channel in FIFO order. The reader task sorts each frame:
  `{"id":…}` is a response and completes the matching `oneshot`; `{"method":…}` with no id is a
  notification and goes to the engine. Ids are a `u32` counter. Kodi echoes the id back. Responses
  and notifications can interleave freely, because they are matched by id and nothing relies on
  frame order.
- **Ordering.** Commands leave in call order on one socket. As far as I know, Kodi's TCP/WS server
  (`CTCPServer`) handles requests on its single thread, so responses also come back in order (not
  verified). One subtle point: `Input.*` calls return as soon as the action has been *queued* to
  the GUI thread, so a poll sent straight after a key press can still see the old focus. The engine
  handles this with sequence numbers (§4), not with timing gaps.
- **Head-of-line risk.** If Kodi's TCP server really is single-threaded, a slow `Files.GetDirectory`
  for a 1,500-movie library would delay key presses queued behind it. So when HTTP is available,
  listing and detail fetches go over HTTP. Kodi's web server uses a thread pool, so the WS stays
  responsive. Measure this on the owner's library before relying on it.
- **Timeouts.** Navigation and transport commands: 2 s. Reads: 4 s. `Files.GetDirectory`: 15 s.
  Three timeouts in a row, or no frame for 25 s while a ping is outstanding, means the link is dead:
  tear down and reconnect. When idle, send a WS ping every 10 s, and `JSONRPC.Ping` if the socket
  layer gives no pong.
- **Reconnect.** Backoff of 0.5, 1, 2, 4, 8 s, then every 15 s, with ±20 % jitter. Reset after
  30 s up. On `System.OnSleep`/`OnQuit` go straight to `Down` and retry slowly. After every
  (re)connect the engine runs a full resync (players, item, timeline, streams, GUI), because
  notifications were missed.
- **Auth.** HTTP uses `Authorization: Basic` from `Credentials`, for both `/jsonrpc` and `/image/`.
  A 401 maps to `Error::Unauthorized`. **The WebSocket has no authentication at all.** Anyone on the
  LAN can control Kodi on 9090 once "Allow remote control from applications on other systems" is
  on. Our credentials protect nothing there, but they are still needed for artwork. The settings
  screen should say this plainly.
- **HTTP fallback** (`TransportPref::Http`, or `Auto` when 9090 refuses): the same engine runs with
  `notifications: false`. While something plays, the engine then polls `Player.GetActivePlayers`
  and the timeline every 2 s, and it re-reads after each of our own commands. Everything still
  works, only more slowly.
- **Crates.** WS: `tokio-tungstenite` 0.24, already in the workspace, without TLS features.
  HTTP: `ureq` 2 with `default-features = false`. It is already in the binary, and the crate calls
  it from `spawn_blocking` with a limit of 4 at a time. It is hidden in `transport/http.rs`, so a
  later move to an async client changes one file. mDNS: `mdns-sd`, which is pure Rust and runs its
  own thread with a channel.

## 3. Kodi API used

| Method (13.5) | Used for |
|---|---|
| `JSONRPC.Version` | Probe; record the API version in `ServerInfo` |
| `JSONRPC.Ping` | Keepalive on an idle socket; cheapest liveness check over HTTP |
| `Application.GetProperties` (`name`, `version`) | Probe and status row: "Kodi 21.3" |
| `Settings.GetSettingValue` | `filelists.ignorethewhensorting` (instead of the hard-coded `ignorearticle: true`), `filelists.showparentdiritems` (whether a ".." row shifts focus indices) |
| `Player.GetActivePlayers` | Resync; pick the `video` player, else the first |
| `Player.GetItem` | Now playing: `title`, `showtitle`, `season`, `episode`, `year`, `runtime`, `art`, `file` (+ `type`/`id`) |
| `Player.GetProperties` | Timeline (`time`, `totaltime`, `speed`); streams (`audiostreams`, `currentaudiostream`, `subtitles`, `currentsubtitle`, `subtitleenabled`); `canseek`, `live` |
| `Player.PlayPause` | Play key (returns the new `Player.Speed`) |
| `Player.Stop` | Stop key |
| `Player.Seek` | Seek bar (`time`), ±N s (`seconds`), the subtitle reload trick |
| `Player.SetAudioStream` | Audio picker (`stream`: index) |
| `Player.SetSubtitle` | Subtitle picker (`subtitle`: index + `enable: true`, or `"off"`) |
| `Player.SetSpeed` | Fast-forward/rewind, if the UI maps them (not mapped today) |
| `Player.Open` | Open or resume a library item (`item: {movieid}` / `{episodeid}`, `options.resume`) |
| `Input.Up/Down/Left/Right/Select/Back/Home/ContextMenu/Info/ShowOSD` | D-pad and menu keys |
| `Input.ExecuteAction` | `pageup`/`pagedown` (channel keys in lists), `skipnext`/`skipprevious`, `osd`, `playpause` |
| `Input.SendText` | Answering `Input.OnInputRequested` with the on-screen keyboard (later) |
| `GUI.GetProperties` (`currentwindow`, `fullscreen`) | Which window Kodi shows: part of the browse poll; context-dependent keys |
| `XBMC.GetInfoLabels` | Browse poll and chapters (InfoLabels below) |
| `XBMC.GetInfoBooleans` | `Container.SortDirection(ascending)`, a language-neutral sort order |
| `Files.GetDirectory` | The wall's list for `Container.FolderPath`, with `media: video`, fields and `List.Sort` |
| `VideoLibrary.GetMovieDetails` / `GetTVShowDetails` / `GetEpisodeDetails` | Refresh one wall entry after `VideoLibrary.OnUpdate` (watched, resume) |

| Notification | Updates |
|---|---|
| `Player.OnPlay`, `Player.OnAVStart` | New item: query item + timeline + streams; `Playback` replaced; `rev.playback` bumps |
| `Player.OnAVChange` | Streams changed: re-read stream properties |
| `Player.OnPause`, `OnResume`, `OnSpeedChanged` | Speed from the payload at once; then re-read `time` to re-anchor `measured_at` |
| `Player.OnSeek` | Position from `data.player.time` (anchored at receive time); no query needed |
| `Player.OnStop` | `playback = None`; browse poll resumes |
| `Player.OnPropertyChanged` | Re-read the changed player's properties (repeat, shuffle, partymode today; harmless) |
| `GUI.OnScreensaverActivated/Deactivated`, `GUI.OnDPMSActivated/Deactivated` | `screensaver`; pause the browse poll while it is on |
| `Input.OnInputRequested` / `Input.OnInputFinished` | `input_request` (the UI may offer its keyboard) |
| `VideoLibrary.OnUpdate` | Refresh that entry in `list` if it is there |
| `VideoLibrary.OnRemove`, `OnScanFinished`, `OnCleanFinished`, `OnRefresh` | Invalidate `list` (re-fetch on next poll) |
| `System.OnQuit`, `OnRestart`, `OnSleep` / `OnWake` | `Link::Down` with slow retry / immediate reconnect |

InfoLabels (free strings, **not** in the introspect schema, so the least verifiable part; they live
in one `InfoLabel` enum in `methods/gui.rs`, with fixture tests): `Container.Content`,
`Container.FolderPath`, `Container.SortMethod`, `Container.SortOrder`, `Container.NumItems`,
`Container.CurrentItem`, `Container.Position`, `Container.Column`, `Container.Row`,
`ListItem.DBID`, `ListItem.DBTYPE`, `Player.Chapter`, `Player.ChapterCount`, `Player.ChapterName`.
Chapters have no `Player.Property.Name` and no notification, so they are polled: every 5 s while
playing and the screen is awake, and right after a seek.

## 4. Data flow

### A key press

```
evdev/keyboard → input::Button → button.kodi() → Some(KodiKey::Nav(Right))
  → handle.nav(Right)               (sync, returns at once)
  → session: engine.apply(Event::KeySent { nav: Right, seq: 41 })
       ├─ Effect::Send(Input.Right)              → writer → WS
       ├─ state.browse.focus = predict(focus, Right, grid)   (confirmed = false)
       └─ Effect::Schedule(BrowsePoll, +80 ms, +200 ms, +450 ms)
  → watch::send(state)  → UI moves the highlight before Kodi answers
  ← {"id":…,"result":"OK"}  → logged; on error → last_error
```

`input.rs` gains `Button::kodi(self, ctx: KeyContext) -> Option<KodiKey>`. `route()` stays for
the volume keys, which still go to Home Assistant through the core. `KeyContext` holds what
`KodiState` knows (playing? which window?), so keys can depend on context, which the core
integration could not do:

| Button | Browsing | Playing (fullscreen video) |
|---|---|---|
| D-pad | `Input.Up/Down/Left/Right`, `Input.Select` | same (Kodi maps them to seek steps and OSD itself) |
| Back / Home | `Input.Back` / `Input.Home` | same |
| Menu | `Input.ContextMenu` | `Input.ShowOSD` |
| Channel ± | `ExecuteAction pageup/pagedown` | `ExecuteAction chapterorbigstepforward/back` |
| Prev / Next | `ExecuteAction skipprevious/skipnext` | same |
| Play | `ExecuteAction playpause` (`play` on a focused item starts it) | `Player.PlayPause` |
| Stop | `Player.Stop` | same |
| Record | none | none |

### A notification

```
WS frame {"method":"Player.OnPause","params":{"data":{"player":{"playerid":1,"speed":0},…}}}
  → notification::parse → Notification::Pause(PlayerEvent{ player: 1, speed: Paused, .. })
  → engine.apply(Event::Notified(..), now)
       ├─ timeline = Timeline { position: position_at(now), speed: Paused, measured_at: now }
       └─ Effect::Send(Query::Timeline(1))   (exact time, to re-anchor)
  → watch::send   (rev.timeline bumps)
  ← GetProperties{time,totaltime,speed} answered; measured_at = send + rtt/2
  → engine.apply(Event::Answer(Query::Timeline, Ok(props)))  → watch::send
```

The UI never extrapolates on its own. It calls `timeline.position_at(Instant::now())` on its frame
or clock tick. The old `Session.position`/`position_at`/`timing` fields and the `ask_position`
round trip were all workarounds for the driver's whole-second, now-and-then reports. They go away.

The title comes from `Player.GetItem` and is keyed by the item's id (or by file for non-library
items). `StableTitle`, `names_stream` and the 7 s settle existed only because the core's driver
flashed track and chapter names into `media_title`. Those flashes never happen here, so all of that
code is deleted, not ported.

### A GUI poll

GUI focus has no notification. The engine decides when to poll. A browse poll is one
`XBMC.GetInfoLabels` with all container labels, plus `GUI.GetProperties` when the window is
unknown. Only one poll is in flight at a time, and polls that pile up are merged.

| Situation | Polling |
|---|---|
| Asleep, screensaver, or nothing to show | none |
| Playing | none for browse; chapters every 5 s |
| Wall visible, idle | every 1 s |
| Wall not visible but Kodi browsing (the wall may appear) | every 3 s |
| After our own nav key | +80 ms, +200 ms, +450 ms, then back to the above; stops early once an answer confirms the prediction |

Stale answers. Each poll carries `seq_at_send`, the last key sequence number at send time. When the
answer arrives:

1. If `seq_at_send < last_key_seq`: the answer comes from before a key press Kodi may not have
   applied yet. It may update the container and list key, but **not** the focus. This replaces the
   old `HOLD` timer and the "ask again in 120 ms" loop.
2. Otherwise, if it disagrees with an unconfirmed prediction and the prediction is less than 400 ms
   old, wait for the next scheduled poll. Kodi's GUI thread may still be animating.
3. Otherwise Kodi's answer wins: `focus.confirmed = true`. Learn `grid.columns` from
   `(Position − Column) / Row` when Kodi gives a row, or from the distance of an up/down move.

The focus index comes from `Container.CurrentItem` (1-based, minus one if `showparentdiritems`
adds a ".." row), checked against `ListItem.DBID`. The old code searched the list for the DBID and
fell back to 0, which jumped to the start whenever the id was missing. If `Container.NumItems`
does not match `list.len()`, the list is stale and is fetched again.

A changed `ListingKey` (folder path + sort) triggers `Files.GetDirectory` with the fields for
`Content::Movies` or `Content::TvShows`. The sort comes from `browse.rs`:
`sort_for(label, ascending) -> Sort`. The GUI's `Container.SortMethod` is a **translated** label
("Year", "Date added"), so the mapping only works for English Kodi. `XBMC.GetInfoBooleans` with
`Container.SortDirection(ascending)` makes the order language-neutral. The method needs an
open-question fix (§6).

## 5. Artwork

- `ArtRef(String)` holds Kodi's raw art string, such as `image://https%3a%2f%2f…%2f/`.
  `Endpoint::image_url(art)` is `http://host:port/image/` plus `art.percent_encode()`.
  **One encoder** in `art.rs` keeps RFC 3986 unreserved characters (`A–Z a–z 0–9 - _ . ~`) and
  writes everything else as `%XX`. **One decoder** is used for the inner URL. The old code had two
  encoders (upper- and lower-case hex) and a decoder that left a trailing `%XX` undecoded
  (`i + 2 < len`).
- **Resizing through the source.** Kodi's `/image/` serves its cached copy of the URL and fetches
  any `image://` URL it has not seen. So a rewritten inner URL still goes through Kodi's cache.
  `art.rs` keeps an ordered list of `Resizer` rules, `fn(&Url, ArtSize) -> Option<Url>`:
  - Jellyfin/Emby: `/Images/…?Format=original` becomes `fillWidth/fillHeight/quality=85`. This is
    today's rule, which cut a poster from ~40 KB to ~5 KB.
  - TMDB: `image.tmdb.org/t/p/original/` becomes `/w185/` or `/w342/` (the usual scraper source).
  - fanart.tv: `assets.fanart.tv/fanart/` becomes `/preview/`.
  - anything else: unchanged.
- **Cache.** `ArtFetcher` holds an in-memory LRU of encoded bytes keyed by final URL, capped at
  12 MB. It merges duplicate in-flight requests, fetches at most 4 at a time, and serves the newest
  request first, so the highlighted poster arrives before far-away ones. Requests for a list that
  is no longer shown are dropped by generation, as now. Decoding and pixel buffers stay in the UI,
  because the crate stays free of `image` and Slint. There is no disk cache, since Kodi already
  caches.

## 6. Setup, discovery, connection test

**Discovery** (`discover.rs`, feature `discovery`):
`discover(timeout) -> impl Stream<Item = Found>`, where
`Found { name, host, addrs, http_port, ws_port, uuid }` merges one Kodi's services by instance name
and `uuid`. From memory of Kodi's `xbmc/network/NetworkServices.cpp` (the introspect spec does not
cover this), with Settings → Services → Zeroconf on, Kodi announces:

- `_xbmc-jsonrpc-h._tcp` and `_http._tcp` on the web-server port, when the web server is on;
- `_xbmc-jsonrpc._tcp` on 9090, when JSON-RPC accepts other systems;
- `_xbmc-events._udp` on 9777 (EventServer; not used);
- `_airplay._tcp`/`_raop._tcp` if AirPlay is on (ignored).

TXT records carry `txtvers=1` and `uuid=<device uuid>`. The instance name is `services.devicename`
("Kodi" by default). **Uncertain:** the exact TXT keys, and whether `_xbmc-jsonrpc._tcp` is
announced when the TCP server listens on localhost only. Check with `avahi-browse -art` on the LAN
before building the merge logic. Store `uuid` as `KodiConfig.instance`, so that a DHCP address
change is fixed by rediscovery instead of manual setup.

**Probe** (`probe.rs`): `probe(&KodiConfig) -> ProbeReport` runs four steps, each with its own
result and a readable diagnosis:

1. TCP connect to `http_port`;
2. HTTP `JSONRPC.Version` with credentials. A 401 means "Wrong username or password". Connection
   refused means "Turn on Settings → Services → Control → Allow remote control via HTTP";
3. WS connect and `JSONRPC.Version`. Refused means "Turn on Allow remote control from applications
   on other systems", and the report notes that HTTP-only mode works but updates more slowly;
4. `Application.GetProperties` to get the name and version for the status row.

A `/image/` fetch of the first art found checks that artwork can be read.

**First run.** With no `kodi` block, the state is `Link::Unconfigured` and the Kodi screen shows
"Set up Kodi". Setup lists discovered Kodis plus "Enter address…", runs the probe on selection,
asks for credentials only after a 401, then saves `settings.json` and calls
`handle.reconfigure(Some(cfg))`. During migration the address can be pre-filled once from the
core's Kodi entity (today's `library::set_kodi` logic), so existing installs need no setup.

**Defaults to know.** On a fresh Kodi 21 install the web server is off, and "applications on other
systems" is off. Those are my understanding of Kodi's defaults; verify on a clean install. So out
of the box, neither 8080 nor 9090 can be reached from the remote, and the probe's diagnoses are the
real first-run instructions. The owner's Kodi already has both on, because the core's Kodi
integration uses them. Port 9090 can only be changed in `advancedsettings.xml`
(`<jsonrpc><tcpport>`), so it is a config field but is not shown on the main setup page.

## 7. Testability

- **Pure and unit-tested:** `wire.rs` (Time ↔ Duration, Speed ↔ int), `notification::parse`,
  `labels.rs` (today's language and label tests move here), `art.rs` (encode/decode round-trips,
  each resizer), `browse.rs` (label parsing, sort mapping, `predict` on a grid with edge rows), and
  above all `engine.rs`: feed `Event`s with fake `Instant`s and assert the `KodiState` and
  `Effect`s that come out.
- **Fixtures:** `tests/fixtures/responses/*.json` and `notifications/*.json` hold frames recorded
  from the owner's Kodi 21.3 with `examples/record.rs`: a film with PGS subtitles, an episode, a
  pause/seek/stop sequence, a 1,000-movie `Files.GetDirectory`. `sessions/*.jsonl` are whole
  recorded sessions that `tests/engine.rs` replays. Each fixture is also checked against the
  vendored schema for the method names and enum values it uses.
- **Schema test** (`tests/schema.rs`): for every `Method` impl, `NAME` exists in `methods`, every
  serialized param name exists, and every required param is sent. Every variant of `PlayerProperty`,
  `InputAction`, `SortMethod` and `GuiProperty` is in the schema's `enums`. Every field we request
  is in `List.Fields.Files` / `Video.Fields.*`. This uses a small hand-written checker, because
  off-the-shelf JSON-Schema validators do not speak Kodi's draft-03 dialect.
- **Fake transport:** `transport::Frames` is just `(mpsc::Sender<Out>, mpsc::Receiver<In>)`, so a
  test can script frames and drive the session with `tokio::time::pause()`. Reconnect and timeout
  tests need no network.
- **`FakeKodi`** answers the same frames as a real Kodi. It has a small library (the demo's films,
  art from local files via `UC_DEMO_*`), a player with a real timeline, streams and subtitle
  switching, and a GUI grid with a column count. It also emits notifications. The UI's demo and
  `--shots` modes start `KodiService::start(Source::Fake(FakeKodi::demo()))` and get the same
  `KodiHandle` as a real Kodi. This replaces `demo_tracks`, the demo's subtitle counter and its
  position tick, and it means previews exercise the real prediction and polling code.

## 8. Migration plan

Each step builds, ships and is useful on its own. A setting `"kodi": { "mode": "core" | "direct" }`,
overridable with `UC_KODI_MODE`, picks the path. The default stays `core` until step 7.

1. **Crate skeleton + schema.** Add `ids`, `wire`, `rpc`, `methods/*`, `notification`, `error`,
   the vendored schema and `tests/schema.rs`. No UI change. *Deletes nothing.*
2. **Transport, session, probe.** Add `transport/*`, `session`, `engine` (connection and resync
   only), `probe`, and `examples/{probe,record}.rs`. Record the first fixtures. *Deletes nothing.*
3. **Now playing, behind `direct`.** The engine handles player notifications, timeline, item,
   streams and chapters. In `direct` mode `Model.title`, `position`, `duration`, `paused`,
   `audio_tracks`, `media_sub` and the poster/backdrop come from `KodiState`. `codec_badge` takes
   `AudioCodec` + channels instead of parsing strings. In `core` mode nothing changes. Once direct
   is the default this deletes: `StableTitle` and its tests, `ask_title`, `ask_position`,
   `names_stream`, `Session.{position, position_at, timing, poster_url}`,
   `Later::{Title, Position}`, the `kodi::base` URL sniffing in `sync_poster`, `kodi::{title,
   position, playing}`, `split_track`, `episode()`, and `audio_format`.
4. **Commands.** Add `handle.rs` and `Button::kodi()`. In `direct` mode, Screen-target keys,
   `Seek`, `OpenPicker` and `PickTrack` go to the handle. Volume keys still go through `Commands`
   to the core. This deletes the `subtitle_ids`, `Later::Subtitles` and `kodi::set_subtitle`
   plumbing, and the `"select_sound_mode"` path. `Commands`/`PRESS_GAP` shrink to the volume
   entity only, and can go once HA volume is checked without the gap.
5. **Poster wall.** Add `browse.rs`, browse polling, prediction, `art.rs` and `ArtFetcher`. The UI
   reads `state.browse` and calls `handle.set_active(..)` in place of `library::allow`. This
   deletes `library.rs`'s `GATE`, `POKE`, `BASE`, `CURSOR`, `LIST`, `predict`, `poke`, `allow`,
   `set_kodi`, `kodi_base` and the loop in `start`, plus `kodi::{browsing, list, smaller, decode,
   image}`. `library.rs` shrinks to `Entry` formatting from `ListEntry` and poster pixels.
6. **Setup and discovery.** Add the settings save, the Kodi setup screen, `discover.rs`, and the
   health row from `Link` instead of the core integration's `kodi_link`. *Deletes:* the Kodi
   branch of `pick_entity`, as far as `direct` is concerned.
7. **Fake Kodi and flip the default.** Demo, preview and `shots.rs` (`kodi_hold` uses
   `handle.set_subtitle`/seek, not `kodi::reload`) run on `FakeKodi`. The default becomes `direct`.
   *Deletes:* `demo_tracks` and the Kodi parts of `run_demo` and `Model::demo`.
8. **Remove the core path** after a week of daily use: `kodi.rs`, the `core` mode switch, the
   Screen-entity handling in `Session`, and `net::Health.kodi_link`'s integration lookup. Then
   disable or uninstall the core's Kodi integration on the remote. This is the user's call; it
   does not touch the warranty flag.

## 9. Open questions and risks

- **Device network sandbox.** `device-contract.md` says LAN is open, but multicast is untested.
  `mdns-sd` needs to bind UDP 5353 and join 224.0.0.251. If the sandbox blocks that, discovery
  silently finds nothing, so manual entry must always be offered. Outbound TCP to 8080/9090 is
  believed fine (the stock UI fetches Kodi art), but a WS from our process has not been tried on
  the device yet.
- **WS head-of-line blocking.** Whether Kodi's TCP server is single-threaded is unverified. Measure
  key latency during a large `Files.GetDirectory` before deciding that HTTP is required for lists.
- **Older Kodi.** Only 13.5 has been introspected. I believe Kodi 19 and 20 report 12.x/13.x, but
  have not checked. `Player.Seek {seconds}`, `Container.CurrentItem` and some stream flags
  (`isoriginal`, `isimpaired`) may be missing on older versions. Gate on probe results and
  `Rpc{code: -32602}` fallbacks, not on version numbers. Vendor a second schema if someone runs
  Kodi 20.
- **Defaults.** The web server and "other systems" are off by default. WS has no auth. Kodi may
  refuse a web server with an empty password on recent versions (unverified), so assume credentials
  are needed.
- **Localised sort labels.** `Container.SortMethod` is translated. English mapping plus
  `SortDirection(ascending)` covers the owner's setup. For other languages either map Kodi's
  localisation strings or check the order by reading `Container.ListItemAbsolute(n).DBID` for a few
  `n` (needs verification).
- **Bitmap subtitle reload.** `Player.Subtitle` has no codec field, so the crate cannot tell PGS or
  VobSub from text subtitles. `SubtitleReload::InPlace` (a seek to the current exact position after
  `Player.SetSubtitle`) is applied on every off→on or track change while playing. Verify that the
  in-place seek is invisible on the owner's Shield or box. `Back(3 s)` stays for recordings, where
  the previous line must show.
- **`Player.OnAVChange` coverage.** It is unclear whether Kodi sends it for subtitle changes. The
  engine always re-reads streams after its own `SetSubtitle`/`SetAudioStream`, so only changes made
  on Kodi's own OSD depend on it.
- **Credentials at rest.** The password sits in plain text in `settings.json` under
  `UC_CONFIG_HOME`. That is acceptable for a LAN media server, but say so in setup.
- **Two clients during migration.** While `core` mode exists, the core's Kodi integration keeps its
  own connection. Kodi accepts several. Running both is harmless, but logs from the two paths can be
  confusing, so tag the crate's logs with a `kodi` target.
