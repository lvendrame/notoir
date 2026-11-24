# 42 — Local Media

**Status:** Partial — native media documents, bounded decoding, viewer controls and playback settings implemented; packaged playback and physical audio acceptance require Windows/macOS/Linux validation

## Behavior

Local media opens through File Open, Explorer, Quick Open, drag/drop, CLI and Open Recent.
Media is read-only and never enters text editing, text comparison, printing, autosave or recovery snapshots.
A media document stores a kind and file identity; it allocates no text rope or undo history.
SVG remains editable text with [Edit/Split/Preview presentation](43-svg.md). URL downloads remain text-only.

Supported image formats: PNG, JPEG, WebP, BMP, TIFF (first page), ICO and GIF.
GIF respects frame delays, transparency, disposal and embedded loop count. Animated WebP is displayed as a still.
Video containers: MP4/MOV, MKV and WebM. Baseline codecs: H.264, HEVC, VP8 and VP9.
Audio: MP3, PCM WAV, FLAC, Ogg Vorbis/Opus, AAC and M4A AAC/ALAC. Container support does not imply every codec.

Image/GIF/video controls use Lucide zoom-in, zoom-out, expand (fit) and scan (actual size),
a percentage input (10–800%), and scrollable/pannable content. Fit never stretches aspect ratio.
Audio/video controls use play/pause, square (stop), volume-2/volume-x, a seek slider,
elapsed and total time, and volume slider. Stop returns to zero. Play after completion restarts.
Controls use the command registry, translated tooltips, keyboard focus and accessible names.
Text zoom remains independent. Unknown durations disable seeking.

## Settings and lifetime

Settings → Media exposes `media.autoplay`, `media.pauseWhenHidden`, `media.pauseOnFocusLoss`,
`media.resumeOnReturn` (all default true) and `media.defaultVolume` (0–100, default 100).
Automatic resume affects only automatic pauses; manual pauses remain paused.
Playback belongs to a document within its window; each split keeps its own zoom/scroll.
A document is visible while any active split displays it. Sessions restore media paused.

Media state, workers, decoders, textures and audio devices are lazy. IO and native-resource destruction
run on workers. Closing the last view signals cancellation without joining on the UI thread.
Hidden paused sessions release their decoded resources after five seconds; they retain position/zoom.
Video has at most two pending presentation frames; PCM queues hold at most one second of stereo audio.
Application-managed surfaces have a shared 256 MiB per-window budget, including decoder working-surface
reservations and CPU/GPU frame copies. FFmpeg internal codec allocations are additional and require profiling.
Dimension/overflow checks precede surface allocation; GPU texture limits are checked before upload.
No entire movie, audio file or animation is read into memory. Seek epochs reject stale presentation frames.
Errors remain in the tab with a translated Retry action. File-watcher changes invalidate the decoder.

## Scenarios

```gherkin
Feature: Media documents
  Scenario: Media tabs have no editable text or save path
    Given a local PNG image
    When it is opened
    Then it has a read-only media view without a text buffer
    And Save and Save As cannot overwrite its bytes
    And zoom commands change only media presentation
    And splitting creates another view of the same document

  Scenario: Media settings expose playback defaults
    Given the Media settings category
    Then autoplay, pause when hidden, pause on focus loss and automatic resume default to enabled
    And default volume is 100 percent
    And returning resumes an automatic pause but leaves a manual pause intact

  Scenario: Closing the final media tab releases the session
    Given an open image tab
    When its final view is closed
    Then the media runtime is released

Feature: Streaming decoders
  Scenario: Image decoding releases its memory
    Given a transparent PNG image
    When a worker decodes it and is closed
    Then its dimensions and alpha values are preserved
    And its surface reservations are released

  Scenario: GIF frames animate without collecting the animation
    Given a GIF with two differently colored frames
    When playback is paused and then resumed
    Then the first frame remains visible while paused
    And the second frame is decoded after playback resumes

  Scenario: Video decoding and seeking are asynchronous
    Given a local WebM video
    When its worker decodes, pauses and seeks
    Then a correctly sized frame is produced
    And the paused playback clock stays stationary
    And a frame from the new seek epoch is produced

  Scenario: Audio streams through a bounded output queue
    Given a PCM WAV sound
    When its worker plays through a deterministic null output
    Then playback advances and reaches the end
    And the PCM queue never exceeds one second
    And playback stops at completion

  Scenario: Oversized media is rejected before surface allocation
    Given invalid dimensions or an exhausted surface budget
    When allocation is requested
    Then the allocation is rejected
    And releasing a reservation restores its budget
```
