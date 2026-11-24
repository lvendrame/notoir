# Images, GIFs, video and sound

Open a local media file from the Explorer, **File → Open File**, Quick Open, drag and drop,
or the command line. It opens as a read-only tab alongside your text documents.

The Lucide magnifier buttons zoom images, GIFs and video. **Fit to pane** fits the picture;
**Actual size** displays its original pixels. Enter a percentage from 10% to 800%, and scroll
or drag the enlarged picture. Media zoom does not change text size. GIFs animate automatically by default and have a Play/Pause control.

Video and audio have Play/Pause, Stop, a playback-position slider, current/total time, Mute and
Volume. Stop returns to the beginning. Press Space with the media tab active to play/pause;
text-input controls keep their normal keyboard behavior. An unknown total time disables seeking.

Under **Settings → Media**, choose autoplay, pause when the tab is hidden, pause when the window
loses focus, automatic resume, and default volume. All playback switches default to enabled.
Returning resumes playback only if leaving caused its pause. A manual pause remains paused.
Restored tabs start paused. A split shares the same playback, with independent picture zoom.

Images: PNG, JPEG, WebP, BMP, TIFF (first page), ICO, GIF. SVG has [Edit, Split and Preview modes](svg.md).
Video: MP4/MOV, MKV, WebM with supported H.264, HEVC, VP8 or VP9 video.
Audio: MP3, PCM WAV, FLAC, Ogg Vorbis/Opus, AAC, and M4A AAC/ALAC.
Animated WebP currently displays a still image. Protected/DRM media is not supported.

Notoir streams media instead of loading every frame or sample into memory. Extremely large
pictures may exceed dimension, GPU or memory limits; close other media tabs and use Retry.
Corrupt files and unavailable sound devices show an error in the tab. Media files cannot be
edited, saved as text, printed, or recovered as text snapshots. No media is uploaded.
