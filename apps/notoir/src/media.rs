//! Lazy local-media sessions. No decoder, audio device, or texture exists until
//! a media view is visible. Workers own IO and native resources; Drop only signals.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::core::AppCore;
use notoir_core::document::MediaKind;
use notoir_core::ids::{DocumentId, ViewId};
use notoir_core::view::ViewKind;

pub const MEMORY_BUDGET: usize = 256 * 1024 * 1024;
const MAX_DIMENSION: u32 = 16384;
const AUDIO_RATE: u32 = 48000;
const AUDIO_SAMPLES: usize = AUDIO_RATE as usize * 2;

#[derive(Default)]
pub struct MediaState {
    pub sessions: HashMap<DocumentId, Session>,
    pub scales: HashMap<ViewId, f32>,
    budget: Arc<AtomicUsize>,
    /// Paths restored from a session must never autoplay.
    pub restored: std::collections::HashSet<DocumentId>,
}
#[derive(Default)]
pub struct Session {
    worker: Option<Worker>,
    pub texture: Option<egui::TextureHandle>,
    surface: Option<Arc<Frame>>,
    pub size: [u32; 2],
    pub duration: Option<f64>,
    pub position: f64,
    pub playing: bool,
    pub volume: f32,
    pub muted: bool,
    pub error: Option<String>,
    pub loading: bool,
    pub ended: bool,
    automatic_pause: bool,
    hidden_since: Option<Instant>,
    initialized: bool,
}
struct Worker {
    shared: Arc<Shared>,
    frames: mpsc::Receiver<Arc<Frame>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}
#[derive(Default)]
struct Info {
    size: [u32; 2],
    duration: Option<f64>,
    error: Option<String>,
    ready: bool,
    ended: bool,
}
struct Shared {
    stop: AtomicBool,
    playing: AtomicBool,
    clock: AtomicU64,
    seek: Mutex<(u64, f64)>,
    gain: AtomicU64,
    info: Mutex<Info>,
    audio: Mutex<std::collections::VecDeque<f32>>,
    audio_done: AtomicBool,
    audio_clock: AtomicBool,
}
impl Shared {
    fn position(&self) -> f64 {
        self.clock.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }
    fn fail(&self, e: impl ToString) {
        self.info.lock().unwrap().error = Some(e.to_string());
        self.playing.store(false, Ordering::Release);
    }
}
struct Lease {
    used: Arc<AtomicUsize>,
    bytes: usize,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
struct Frame {
    epoch: u64,
    size: [usize; 2],
    rgba: Vec<u8>,
    _lease: Lease,
}
fn checked_bytes(w: u32, h: u32) -> Result<usize, String> {
    if w == 0 || h == 0 || w > MAX_DIMENSION || h > MAX_DIMENSION {
        return Err("media:tooLarge".into());
    }
    (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| *n <= MEMORY_BUDGET / 4)
        .ok_or_else(|| "media:tooLarge".into())
}
fn reserve(budget: &Arc<AtomicUsize>, bytes: usize) -> Result<Lease, String> {
    budget
        .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            n.checked_add(bytes).filter(|n| *n <= MEMORY_BUDGET)
        })
        .map_err(|_| "media:memoryLimit".to_owned())?;
    Ok(Lease {
        used: budget.clone(),
        bytes,
    })
}
fn send_frame(tx: &mpsc::SyncSender<Arc<Frame>>, frame: Arc<Frame>, shared: &Shared) -> bool {
    let mut frame = frame;
    while !shared.stop.load(Ordering::Acquire) {
        if frame.epoch != shared.seek.lock().unwrap().0 {
            return true;
        }
        match tx.try_send(frame) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Full(f)) => {
                frame = f;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return false,
        }
    }
    false
}
impl Worker {
    fn start(
        path: PathBuf,
        kind: MediaKind,
        budget: Arc<AtomicUsize>,
        playing: bool,
        position: f64,
        gain: f32,
        poster: crate::io::Poster,
    ) -> Self {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            playing: AtomicBool::new(playing),
            clock: AtomicU64::new((position * 1_000_000.0) as u64),
            seek: Mutex::new((0, position)),
            gain: AtomicU64::new(gain.to_bits() as u64),
            info: Mutex::new(Info::default()),
            audio: Mutex::new(std::collections::VecDeque::new()),
            audio_done: AtomicBool::new(false),
            audio_clock: AtomicBool::new(false),
        });
        let (tx, frames) = mpsc::sync_channel(2);
        let s = shared.clone();
        let spawn = std::thread::Builder::new()
            .name("notoir-media".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match kind {
                        MediaKind::Image | MediaKind::Gif => {
                            images(&path, kind, &budget, &tx, &s, &poster)
                        }
                        _ => audiovisual(&path, kind, &budget, &tx, &s, &poster),
                    }));
                match result {
                    Ok(Err(e)) => s.fail(e),
                    Err(_) => s.fail("media:decodeFailed"),
                    _ => {}
                }
                poster.wake();
            });
        if let Err(e) = spawn {
            shared.fail(e);
        }
        Self { shared, frames }
    }
}

fn images(
    path: &Path,
    kind: MediaKind,
    budget: &Arc<AtomicUsize>,
    tx: &mpsc::SyncSender<Arc<Frame>>,
    s: &Shared,
    poster: &crate::io::Poster,
) -> Result<(), String> {
    use image::{AnimationDecoder, ImageDecoder};
    let reader = || {
        image::ImageReader::open(path)
            .map_err(|e| e.to_string())?
            .with_guessed_format()
            .map_err(|e| e.to_string())
    };
    let (w, h) = reader()?.into_dimensions().map_err(|e| e.to_string())?;
    let bytes = checked_bytes(w, h)?;
    // Reserve decoder working surfaces before allocation; GIF disposal may keep
    // a previous canvas. Each delivered frame separately accounts for CPU + GPU.
    let _decode = reserve(budget, bytes * 2)?;
    {
        let mut info = s.info.lock().unwrap();
        info.size = [w, h];
    }
    if kind == MediaKind::Image {
        let lease = reserve(budget, bytes * 2)?;
        let mut input = reader()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some((bytes * 2) as u64);
        limits.max_image_width = Some(MAX_DIMENSION);
        limits.max_image_height = Some(MAX_DIMENSION);
        input.limits(limits);
        let image = input.decode().map_err(|e| e.to_string())?.into_rgba8();
        if image.dimensions() != (w, h) {
            return Err("media:decodeFailed".into());
        }
        let frame = Arc::new(Frame {
            epoch: 0,
            size: [w as usize, h as usize],
            rgba: image.into_raw(),
            _lease: lease,
        });
        send_frame(tx, frame, s);
        s.info.lock().unwrap().ready = true;
        poster.wake();
        return Ok(());
    }
    let mut loops = 0;
    loop {
        let input = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
        let mut decoder = image::codecs::gif::GifDecoder::new(input).map_err(|e| e.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some((bytes * 2) as u64);
        limits.max_image_width = Some(MAX_DIMENSION);
        limits.max_image_height = Some(MAX_DIMENSION);
        decoder.set_limits(limits).map_err(|e| e.to_string())?;
        if decoder.dimensions() != (w, h) {
            return Err("media:decodeFailed".into());
        }
        let count = decoder.loop_count();
        for frame in decoder.into_frames() {
            if s.stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let lease = reserve(budget, bytes * 2)?;
            let frame = frame.map_err(|e| e.to_string())?;
            let (num, den) = frame.delay().numer_denom_ms();
            let delay =
                Duration::from_secs_f64((num as f64 / den.max(1) as f64 / 1000.0).max(0.01));
            let frame = Arc::new(Frame {
                epoch: 0,
                size: [w as usize, h as usize],
                rgba: frame.into_buffer().into_raw(),
                _lease: lease,
            });
            if !send_frame(tx, frame, s) {
                return Ok(());
            }
            s.info.lock().unwrap().ready = true;
            poster.wake();
            let mut remaining = delay;
            let mut last = Instant::now();
            while !remaining.is_zero() {
                if s.stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(5));
                let elapsed = last.elapsed();
                last = Instant::now();
                if s.playing.load(Ordering::Acquire) {
                    remaining = remaining.saturating_sub(elapsed);
                }
            }
        }
        loops += 1;
        if let image::metadata::LoopCount::Finite(n) = count
            && loops >= n.get()
        {
            break;
        }
    }
    s.info.lock().unwrap().ended = true;
    s.playing.store(false, Ordering::Release);
    Ok(())
}

fn input(path: &Path, s: &Arc<Shared>) -> Result<ffmpeg_next::format::context::Input, String> {
    let cancel = s.clone();
    let mut options = ffmpeg_next::Dictionary::new();
    options.set("protocol_whitelist", "file");
    ffmpeg_next::format::input_with_interrupt_and_dictionary(
        path,
        move || cancel.stop.load(Ordering::Acquire),
        options,
    )
    .map_err(|e| e.to_string())
}
fn audiovisual(
    path: &Path,
    kind: MediaKind,
    budget: &Arc<AtomicUsize>,
    tx: &mpsc::SyncSender<Arc<Frame>>,
    s: &Arc<Shared>,
    poster: &crate::io::Poster,
) -> Result<(), String> {
    use ffmpeg_next as f;
    f::init().map_err(|e| e.to_string())?;
    let mut input = input(path, s)?;
    let audio = input.streams().best(f::media::Type::Audio).is_some();
    let duration = (input.duration() > 0).then(|| input.duration() as f64 / 1_000_000.0);
    s.info.lock().unwrap().duration = duration;
    if audio {
        let p = path.to_path_buf();
        let shared = s.clone();
        let wake = poster.clone();
        let audio_budget = budget.clone();
        std::thread::Builder::new()
            .name("notoir-media-audio".into())
            .spawn(move || {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    audio_decode(&p, &shared, kind == MediaKind::Audio, &audio_budget)
                })) {
                    Ok(Err(e)) => shared.fail(e),
                    Err(_) => shared.fail("media:decodeFailed"),
                    _ => {}
                }
                wake.wake();
            })
            .map_err(|e| e.to_string())?;
    } else {
        s.audio_done.store(true, Ordering::Release);
    }
    if kind == MediaKind::Audio {
        if !audio {
            return Err("media:noAudio".into());
        }
        s.info.lock().unwrap().ready = true;
        poster.wake();
        while !s.stop.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(10));
        }
        return Ok(());
    }
    let stream = input
        .streams()
        .best(f::media::Type::Video)
        .ok_or_else(|| "media:noVideo".to_owned())?;
    let index = stream.index();
    let time_base: f64 = stream.time_base().into();
    let offset = if stream.start_time() == f::ffi::AV_NOPTS_VALUE {
        0.0
    } else {
        stream.start_time() as f64 * time_base
    };
    let context = f::codec::context::Context::from_parameters(stream.parameters())
        .map_err(|e| e.to_string())?;
    let mut decoder = context.decoder().video().map_err(|e| e.to_string())?;
    let (w, h) = (decoder.width(), decoder.height());
    let bytes = checked_bytes(w, h)?;
    let _working = reserve(budget, bytes * 2)?;
    let mut scaler = f::software::scaling::Context::get(
        decoder.format(),
        w,
        h,
        f::format::Pixel::RGBA,
        w,
        h,
        f::software::scaling::Flags::BILINEAR,
    )
    .map_err(|e| e.to_string())?;
    {
        let mut i = s.info.lock().unwrap();
        i.size = [w, h];
        i.ready = true;
    }
    poster.wake();
    let mut epoch = u64::MAX;
    let mut eof = false;
    let mut last = Instant::now();
    let mut decoded = f::frame::Video::empty();
    let mut converted = f::frame::Video::empty();
    loop {
        if s.stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let seek = *s.seek.lock().unwrap();
        if seek.0 != epoch {
            if seek.1 > 0.0 || epoch != u64::MAX {
                input
                    .seek(((seek.1 + offset) * 1_000_000.0) as i64, ..)
                    .map_err(|e| e.to_string())?;
            }
            decoder.flush();
            epoch = seek.0;
            eof = false;
        }
        if !s.audio_clock.load(Ordering::Acquire)
            && (!audio || s.audio_done.load(Ordering::Acquire))
            && s.playing.load(Ordering::Acquire)
        {
            s.clock
                .fetch_add(last.elapsed().as_micros() as u64, Ordering::Relaxed);
        }
        last = Instant::now();
        if eof {
            if s.audio_done.load(Ordering::Acquire)
                && duration.is_none_or(|d| s.position() >= d - 0.05)
            {
                s.info.lock().unwrap().ended = true;
                s.playing.store(false, Ordering::Release);
                poster.wake();
            }
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        match decoder.receive_frame(&mut decoded) {
            Ok(()) => {
                let pts = decoded
                    .timestamp()
                    .or(decoded.pts())
                    .map_or(s.position(), |p| (p as f64 * time_base - offset).max(0.0));
                // Wait for presentation without blocking the independent audio decoder.
                while pts > s.position() + 0.005
                    || (!s.playing.load(Ordering::Acquire) && pts > seek.1 + 0.001)
                {
                    if s.stop.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    if s.seek.lock().unwrap().0 != epoch {
                        break;
                    }
                    if (!audio || s.audio_done.load(Ordering::Acquire))
                        && s.playing.load(Ordering::Acquire)
                    {
                        s.clock
                            .fetch_add(last.elapsed().as_micros() as u64, Ordering::Relaxed);
                    }
                    last = Instant::now();
                    std::thread::sleep(Duration::from_millis(5));
                }
                if s.seek.lock().unwrap().0 != epoch
                    || pts < seek.1 - 0.05
                    || pts < s.position() - 0.15
                {
                    continue;
                }
                let lease = reserve(budget, bytes * 2)?;
                scaler
                    .run(&decoded, &mut converted)
                    .map_err(|e| e.to_string())?;
                let mut rgba = Vec::with_capacity(bytes);
                for row in 0..h as usize {
                    let start = row * converted.stride(0);
                    rgba.extend_from_slice(&converted.data(0)[start..start + w as usize * 4]);
                }
                if !send_frame(
                    tx,
                    Arc::new(Frame {
                        epoch,
                        size: [w as usize, h as usize],
                        rgba,
                        _lease: lease,
                    }),
                    s,
                ) {
                    return Ok(());
                }
                poster.wake();
            }
            Err(f::Error::Eof) => eof = true,
            Err(f::Error::Other { errno }) if errno == f::error::EAGAIN => {
                let mut packet = f::Packet::empty();
                match packet.read(&mut input) {
                    Ok(()) if packet.stream() == index => {
                        decoder.send_packet(&packet).map_err(|e| e.to_string())?
                    }
                    Ok(()) => {}
                    Err(f::Error::Eof) => {
                        decoder.send_eof().map_err(|e| e.to_string())?;
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

struct AudioSource {
    shared: Arc<Shared>,
    fractional: u32,
}
impl Iterator for AudioSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        let s = &self.shared;
        if s.stop.load(Ordering::Acquire) {
            return None;
        }
        if !s.playing.load(Ordering::Acquire) {
            return Some(0.0);
        }
        let Ok(mut queue) = s.audio.try_lock() else {
            return Some(0.0);
        };
        let Some(sample) = queue.pop_front() else {
            return Some(0.0);
        };
        // Exactly 1e6 microseconds for each second of stereo samples.
        self.fractional += 1_000_000;
        let delta = self.fractional / AUDIO_SAMPLES as u32;
        self.fractional %= AUDIO_SAMPLES as u32;
        s.clock.fetch_add(delta as u64, Ordering::Relaxed);
        Some(sample * f32::from_bits(s.gain.load(Ordering::Relaxed) as u32))
    }
}
impl rodio::Source for AudioSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        2.try_into().unwrap()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        AUDIO_RATE.try_into().unwrap()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}
fn audio_decode(
    path: &Path,
    s: &Arc<Shared>,
    audio_only: bool,
    budget: &Arc<AtomicUsize>,
) -> Result<(), String> {
    let _pcm_budget = reserve(budget, AUDIO_SAMPLES * std::mem::size_of::<f32>())?;
    use ffmpeg_next as f;
    let mut input = input(path, s)?;
    let stream = input
        .streams()
        .best(f::media::Type::Audio)
        .ok_or_else(|| "media:noAudio".to_owned())?;
    let index = stream.index();
    let time_base: f64 = stream.time_base().into();
    let offset = if stream.start_time() == f::ffi::AV_NOPTS_VALUE {
        0.0
    } else {
        stream.start_time() as f64 * time_base
    };
    let context = f::codec::context::Context::from_parameters(stream.parameters())
        .map_err(|e| e.to_string())?;
    let mut decoder = context.decoder().audio().map_err(|e| e.to_string())?;
    let layout = if decoder.channel_layout().is_empty() {
        f::ChannelLayout::default(decoder.channels() as i32)
    } else {
        decoder.channel_layout()
    };
    let input_format = decoder.format();
    let input_rate = decoder.rate();
    let make_resampler = || {
        f::software::resampling::Context::get(
            input_format,
            layout,
            input_rate,
            f::format::Sample::F32(f::format::sample::Type::Packed),
            f::ChannelLayout::STEREO,
            AUDIO_RATE,
        )
        .map_err(|e| e.to_string())
    };
    let mut resampler = make_resampler()?;
    // Unit tests exercise the actual decoder through a deterministic null sink.
    let device = if cfg!(test) {
        None
    } else {
        Some(rodio::DeviceSinkBuilder::open_default_sink().map_err(|e| e.to_string())?)
    };
    let player = device
        .as_ref()
        .map(|d| rodio::Player::connect_new(d.mixer()));
    {
        let mut q = s.audio.lock().unwrap();
        q.reserve(AUDIO_SAMPLES);
    }
    if let Some(player) = &player {
        player.append(AudioSource {
            shared: s.clone(),
            fractional: 0,
        });
    }
    #[cfg(test)]
    {
        let shared = s.clone();
        std::thread::spawn(move || {
            let mut source = AudioSource {
                shared,
                fractional: 0,
            };
            while !source.shared.stop.load(Ordering::Acquire) {
                for _ in 0..960 {
                    let _ = source.next();
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
    }
    s.audio_clock.store(true, Ordering::Release);
    let mut epoch = u64::MAX;
    let mut eof = false;
    let mut decoded = f::frame::Audio::empty();
    loop {
        if s.stop.load(Ordering::Acquire) {
            return Ok(());
        }
        let seek = *s.seek.lock().unwrap();
        if epoch != seek.0 {
            if seek.1 > 0.0 || epoch != u64::MAX {
                input
                    .seek(((seek.1 + offset) * 1_000_000.0) as i64, ..)
                    .map_err(|e| e.to_string())?;
            }
            decoder.flush();
            resampler = make_resampler()?;
            eof = false;
            epoch = seek.0;
            let request = s.seek.lock().unwrap();
            if request.0 != epoch {
                continue;
            }
            let mut q = s.audio.lock().unwrap();
            q.clear();
            s.clock
                .store((seek.1 * 1_000_000.0) as u64, Ordering::Release);
            s.audio_done.store(false, Ordering::Release);
            s.audio_clock.store(true, Ordering::Release);
        }
        if eof {
            if s.audio.lock().unwrap().is_empty() {
                s.audio_done.store(true, Ordering::Release);
                s.audio_clock.store(false, Ordering::Release);
                if audio_only {
                    s.info.lock().unwrap().ended = true;
                    s.playing.store(false, Ordering::Release);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        if !s.playing.load(Ordering::Acquire) || s.audio.lock().unwrap().len() >= AUDIO_SAMPLES / 2
        {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        match decoder.receive_frame(&mut decoded) {
            Ok(()) => {
                if decoded.channel_layout().is_empty() {
                    decoded.set_channel_layout(layout);
                }
                let mut output = f::frame::Audio::empty();
                resampler
                    .run(&decoded, &mut output)
                    .map_err(|e| e.to_string())?;
                let pts = decoded
                    .timestamp()
                    .or(decoded.pts())
                    .map_or(seek.1, |p| (p as f64 * time_base - offset).max(0.0));
                let skip = ((seek.1 - pts).max(0.0) * AUDIO_RATE as f64) as usize;
                let samples = output.plane::<(f32, f32)>(0);
                let mut next = skip.min(output.samples());
                while next < output.samples() {
                    if s.stop.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    let generation = s.seek.lock().unwrap();
                    if generation.0 != epoch {
                        break;
                    }
                    let mut queue = s.audio.lock().unwrap();
                    let count = ((AUDIO_SAMPLES - queue.len()) / 2).min(output.samples() - next);
                    for &(l, r) in &samples[next..next + count] {
                        queue.push_back(l);
                        queue.push_back(r);
                    }
                    next += count;
                    drop(queue);
                    drop(generation);
                    if count == 0 {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
            Err(f::Error::Eof) => eof = true,
            Err(f::Error::Other { errno }) if errno == f::error::EAGAIN => {
                let mut packet = f::Packet::empty();
                match packet.read(&mut input) {
                    Ok(()) if packet.stream() == index => {
                        decoder.send_packet(&packet).map_err(|e| e.to_string())?
                    }
                    Ok(()) => {}
                    Err(f::Error::Eof) => {
                        decoder.send_eof().map_err(|e| e.to_string())?;
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

impl AppCore {
    /// Called once per frame, including when no media UI is rendered.
    pub fn tick_media(&mut self, ctx: &egui::Context, focused: bool) {
        let docs: Vec<_> = self
            .workbench
            .documents
            .values()
            .filter_map(|d| Some((d.id, d.media_kind()?, d.path()?.to_path_buf())))
            .collect();
        if docs.is_empty() {
            self.media = None;
            return;
        }
        let autoplay = self.settings.bool("media.autoplay", None);
        let pause_hidden = self.settings.bool("media.pauseWhenHidden", None);
        let pause_focus = self.settings.bool("media.pauseOnFocusLoss", None);
        let resume = self.settings.bool("media.resumeOnReturn", None);
        let volume = self.settings.int("media.defaultVolume", None) as f32 / 100.0;
        let state = self
            .media
            .get_or_insert_with(|| Box::new(MediaState::default()));
        state
            .sessions
            .retain(|id, _| docs.iter().any(|d| d.0 == *id));
        state
            .scales
            .retain(|v, _| self.workbench.view(*v).is_some());
        state.restored.retain(|id| docs.iter().any(|d| d.0 == *id));
        for (id, kind, path) in docs {
            let visible = self.workbench.layout.groups_in_order().iter().any(|g| {
                self.workbench
                    .layout
                    .group(*g)
                    .and_then(|g| g.active)
                    .and_then(|v| self.workbench.view(v))
                    .is_some_and(|v| v.document_id == id)
            });
            if !visible && !state.sessions.contains_key(&id) {
                continue;
            }
            let session = state.sessions.entry(id).or_default();
            if !session.initialized {
                session.volume = volume;
                session.playing =
                    kind != MediaKind::Image && autoplay && !state.restored.contains(&id);
                session.initialized = true;
            }
            let should_pause =
                (!visible && (pause_hidden || kind == MediaKind::Gif)) || (!focused && pause_focus);
            if should_pause && session.playing {
                session.playing = false;
                session.automatic_pause = true;
            }
            if !should_pause && session.automatic_pause {
                session.playing = resume;
                session.automatic_pause = false;
            }
            if visible || session.playing {
                session.hidden_since = None;
            } else {
                session.hidden_since.get_or_insert_with(Instant::now);
            }
            if session
                .hidden_since
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(5))
            {
                session.worker = None;
                session.texture = None;
                session.surface = None;
                continue;
            }
            if session.worker.is_none() && (visible || session.playing) && session.error.is_none() {
                session.loading = true;
                session.worker = Some(Worker::start(
                    path,
                    kind,
                    state.budget.clone(),
                    session.playing,
                    session.position,
                    if session.muted { 0.0 } else { session.volume },
                    self.io.poster(),
                ));
            }
            if let Some(worker) = &session.worker {
                let running = {
                    let info = worker.shared.info.lock().unwrap();
                    !info.ended && info.error.is_none()
                };
                if running {
                    worker
                        .shared
                        .playing
                        .store(session.playing, Ordering::Release);
                }
                worker.shared.gain.store(
                    (if session.muted {
                        0.0_f32
                    } else {
                        session.volume
                    })
                    .to_bits() as u64,
                    Ordering::Relaxed,
                );
                session.position = worker.shared.position();
                let info = worker.shared.info.lock().unwrap();
                session.duration = info.duration;
                session.size = info.size;
                session.error = info.error.clone();
                session.ended = info.ended;
                session.loading = !info.ready && info.error.is_none();
                if info.ended || info.error.is_some() {
                    session.playing = false;
                }
                drop(info);
                if let Some(frame) = worker
                    .frames
                    .try_iter()
                    .filter(|f| f.epoch == worker.shared.seek.lock().unwrap().0)
                    .last()
                {
                    let image = egui::ColorImage::from_rgba_unmultiplied(frame.size, &frame.rgba);
                    if frame
                        .size
                        .iter()
                        .any(|n| *n > ctx.input(|i| i.max_texture_side))
                    {
                        session.error = Some("media:tooLarge".into());
                        session.playing = false;
                    } else {
                        if let Some(texture) = &mut session.texture {
                            texture.set(image, egui::TextureOptions::LINEAR);
                        } else {
                            session.texture = Some(ctx.load_texture(
                                format!("media-{}", id.0),
                                image,
                                egui::TextureOptions::LINEAR,
                            ));
                        }
                        session.surface = Some(frame);
                    }
                }
            }
            if session.error.is_some() {
                session.worker = None;
                session.loading = false;
                session.automatic_pause = false;
            }
            if session.playing || session.loading || session.hidden_since.is_some() {
                ctx.request_repaint_after(Duration::from_millis(if session.playing {
                    16
                } else {
                    100
                }));
            }
        }
    }
    pub fn media_action(&mut self, inv: &notoir_core::command::CommandInvocation) {
        let view = inv
            .args
            .get("view")
            .and_then(|v| v.as_u64())
            .map(ViewId)
            .or_else(|| self.workbench.active_view());
        let Some(view) = view else {
            return;
        };
        let Some(v) = self.workbench.view(view) else {
            return;
        };
        if v.kind != ViewKind::Media {
            return;
        }
        let id = v.document_id;
        let kind = self.workbench.document(id).and_then(|d| d.media_kind());
        let effective_zoom = self
            .media
            .as_ref()
            .and_then(|m| m.scales.get(&view))
            .copied()
            .unwrap_or(100.0);
        let op = inv.id.as_str().strip_prefix("media.").unwrap_or("");
        if matches!(op, "zoomIn" | "zoomOut" | "actualSize" | "fit" | "setZoom") {
            let v = self.workbench.views.get_mut(view).unwrap();
            v.zoom = match op {
                "fit" => None,
                "actualSize" => Some(100.0),
                "setZoom" => inv
                    .args
                    .get("value")
                    .and_then(|v| v.as_f64())
                    .filter(|n| n.is_finite())
                    .map(|n| n.clamp(10.0, 800.0) as f32),
                "zoomIn" => Some((v.zoom.unwrap_or(effective_zoom) * 1.25).clamp(10.0, 800.0)),
                _ => Some((v.zoom.unwrap_or(effective_zoom) / 1.25).clamp(10.0, 800.0)),
            };
            return;
        }
        let Some(session) = self.media.as_mut().and_then(|m| m.sessions.get_mut(&id)) else {
            return;
        };
        match op {
            "playPause" => {
                session.playing = !session.playing;
                session.automatic_pause = false;
                if session.ended {
                    if kind == Some(MediaKind::Gif) {
                        session.worker = None;
                        session.texture = None;
                        session.surface = None;
                        session.ended = false;
                    } else {
                        session.seek(0.0);
                    }
                }
            }
            "stop" => {
                session.playing = false;
                session.automatic_pause = false;
                session.seek(0.0);
            }
            "seek" => {
                if let Some(p) = inv
                    .args
                    .get("value")
                    .and_then(|v| v.as_f64())
                    .filter(|p| p.is_finite())
                {
                    session.seek(p);
                }
            }
            "mute" => session.muted = !session.muted,
            "volume" => {
                if let Some(v) = inv
                    .args
                    .get("value")
                    .and_then(|v| v.as_f64())
                    .filter(|v| v.is_finite())
                {
                    session.volume = v.clamp(0.0, 1.0) as f32;
                }
            }
            "retry" => {
                session.worker = None;
                session.error = None;
                session.texture = None;
                session.surface = None;
            }
            _ => {}
        }
    }
    pub fn reload_media(&mut self, doc: DocumentId) {
        if let Some(session) = self.media.as_mut().and_then(|m| m.sessions.get_mut(&doc)) {
            session.worker = None;
            session.texture = None;
            session.surface = None;
            session.error = None;
        }
    }
}
impl Session {
    fn seek(&mut self, position: f64) {
        self.position = position.clamp(0.0, self.duration.unwrap_or(f64::MAX));
        self.ended = false;
        if let Some(w) = &mut self.worker {
            // Drop queued frames immediately; the worker tags its presentation
            // with the seek epoch before producing anything else.
            for _ in w.frames.try_iter() {}
            let mut seek = w.shared.seek.lock().unwrap();
            seek.0 += 1;
            seek.1 = self.position;
            w.shared.audio.lock().unwrap().clear();
            w.shared
                .clock
                .store((self.position * 1_000_000.0) as u64, Ordering::Release);
            w.shared.info.lock().unwrap().ended = false;
        }
    }
}

pub fn time_label(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn worker(path: PathBuf, kind: MediaKind, playing: bool) -> (Worker, Arc<AtomicUsize>) {
        let q = crate::io::IoQueue::start(|| {});
        let budget = Arc::new(AtomicUsize::new(0));
        (
            Worker::start(path, kind, budget.clone(), playing, 0.0, 1.0, q.poster()),
            budget,
        )
    }
    fn wait(w: &Worker, predicate: impl Fn(&Info) -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let info = w.shared.info.lock().unwrap();
            assert!(info.error.is_none(), "{:?}", info.error);
            if predicate(&info) {
                return;
            }
            drop(info);
            assert!(Instant::now() < until, "media worker timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    // spec 42: "Image decoding releases its memory"
    #[test]
    fn image_decode_and_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.png");
        image::RgbaImage::from_pixel(8, 4, image::Rgba([255, 0, 0, 128]))
            .save(&path)
            .unwrap();
        let (worker, budget) = worker(path, MediaKind::Image, false);
        wait(&worker, |i| i.ready);
        let frame = worker.frames.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(frame.size, [8, 4]);
        assert_eq!(&frame.rgba[..4], &[255, 0, 0, 128]);
        drop(worker);
        drop(frame);
        let until = Instant::now() + Duration::from_secs(2);
        while budget.load(Ordering::Acquire) != 0 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(budget.load(Ordering::Acquire), 0);
    }
    // spec 42: "GIF frames animate without collecting the animation"
    #[test]
    fn gif_streams_and_pauses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.gif");
        let mut encoder =
            image::codecs::gif::GifEncoder::new(std::fs::File::create(&path).unwrap());
        encoder
            .set_repeat(image::codecs::gif::Repeat::Finite(0))
            .unwrap();
        for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
            encoder
                .encode_frame(image::Frame::from_parts(
                    image::RgbaImage::from_pixel(4, 4, image::Rgba(color)),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(50, 1),
                ))
                .unwrap();
        }
        drop(encoder);
        let (w, _) = worker(path, MediaKind::Gif, false);
        wait(&w, |i| i.ready);
        let a = w.frames.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(&a.rgba[..4], &[255, 0, 0, 255]);
        assert!(w.frames.recv_timeout(Duration::from_millis(90)).is_err());
        w.shared.playing.store(true, Ordering::Release);
        let b = w.frames.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(&b.rgba[..4], &[0, 255, 0, 255]);
    }
    // spec 42: "Video decoding and seeking are asynchronous"
    #[test]
    fn video_decodes_pauses_seeks_and_stops() {
        let (w, _) = worker(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/media/video.webm"),
            MediaKind::Video,
            true,
        );
        wait(&w, |i| i.ready);
        let frame = w.frames.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(frame.size, [64, 48]);
        w.shared.playing.store(false, Ordering::Release);
        std::thread::sleep(Duration::from_millis(20));
        let pos = w.shared.position();
        std::thread::sleep(Duration::from_millis(60));
        assert!((w.shared.position() - pos).abs() < 0.02);
        for _ in w.frames.try_iter() {}
        *w.shared.seek.lock().unwrap() = (1, 0.3);
        w.shared.clock.store(300000, Ordering::Release);
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(frame) = w.frames.recv_timeout(Duration::from_millis(100))
                && frame.epoch == 1
            {
                break;
            }
            assert!(Instant::now() < until, "seek produced no frame");
        }
        drop(w);
    }
    // spec 42: "Audio streams through a bounded output queue"
    #[test]
    fn audio_decodes_through_null_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        let samples = 4800_u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + samples * 2).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&48000_u32.to_le_bytes());
        bytes.extend_from_slice(&96000_u32.to_le_bytes());
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(samples * 2).to_le_bytes());
        for i in 0..samples {
            let value =
                ((i as f32 / 48000.0 * 440.0 * std::f32::consts::TAU).sin() * 10000.0) as i16;
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        let (w, _) = worker(path, MediaKind::Audio, true);
        wait(&w, |i| i.ended);
        assert!(w.shared.position() > 0.08);
        assert!(w.shared.audio.lock().unwrap().len() <= AUDIO_SAMPLES);
        assert!(!w.shared.playing.load(Ordering::Acquire));
    }
    #[test]
    fn video_and_audio_finish_together() {
        let (w, _) = worker(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/media/video-audio.mp4"),
            MediaKind::Video,
            true,
        );
        let until = Instant::now() + Duration::from_secs(5);
        let mut frames = 0;
        loop {
            frames += w.frames.try_iter().count();
            let info = w.shared.info.lock().unwrap();
            assert!(info.error.is_none(), "{:?}", info.error);
            if info.ended {
                break;
            }
            drop(info);
            assert!(
                Instant::now() < until,
                "A/V playback did not finish at {}",
                w.shared.position()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(frames >= 3);
        assert!(w.shared.position() >= 0.5);
        assert!(w.shared.audio_done.load(Ordering::Acquire));
    }
    // spec 42: "Oversized media is rejected before surface allocation"
    #[test]
    fn limits_are_checked_before_allocation() {
        assert!(checked_bytes(u32::MAX, 10).is_err());
        assert!(checked_bytes(0, 10).is_err());
        let budget = Arc::new(AtomicUsize::new(0));
        let lease = reserve(&budget, MEMORY_BUDGET).unwrap();
        assert!(reserve(&budget, 1).is_err());
        drop(lease);
        assert_eq!(budget.load(Ordering::Acquire), 0);
        assert_eq!(time_label(3661.0), "1:01:01");
    }
}
