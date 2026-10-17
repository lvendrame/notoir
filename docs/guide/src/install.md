# Installing Notoir

Download the installer for your system from the
[latest release](https://github.com/lvendrame/notoir/releases/latest):

| System | File |
|---|---|
| macOS 11 or later (Apple Silicon and Intel) | `Notoir_<version>_universal.dmg` |
| Windows 10 and 11 | `notoir_<version>_x64_en-US.msi` or `notoir_<version>_x64-setup.exe` |
| Linux (X11 and Wayland) | `notoir_<version>_amd64.deb` or `notoir_<version>_x86_64.AppImage` |

Each release also has `SHA256SUMS.txt` with the checksum of every file.

## macOS

1. Open the `.dmg` and drag **Notoir** to **Applications**.
2. Release builds are not code-signed yet, so the first launch says *"Notoir" Not Opened*. Clear the download
   quarantine once:

   ```sh
   xattr -dr com.apple.quarantine /Applications/Notoir.app
   ```

   or open **System Settings › Privacy & Security** and click **Open Anyway**.

## Windows

Run the `.msi` or the `-setup.exe` (which installs for the current user only, without administrator rights). If SmartScreen says
*"Windows protected your PC"*, click **More info › Run anyway**. The builds are not code-signed yet.

## Linux

- **Debian and Ubuntu:** `sudo apt install ./notoir_<version>_amd64.deb`. This also installs the `notoir` command.
- **AppImage:** make it executable and run it:

  ```sh
  chmod +x notoir_*.AppImage
  ./notoir_*.AppImage
  ```

## The `notoir` command

The Linux package puts `notoir` on your `PATH`. On macOS, on Windows and with the AppImage, use
**Help › Install 'notoir' Command in PATH** (and **Help › Uninstall 'notoir' Command** to remove it). See
[Command line](cli.md).

## Updates

Notoir checks for a new version at most once a day and tells you when one is available. Use
**Help › Check for Updates…** to check now. The `update.mode` setting chooses the behaviour:

| `update.mode` | What happens |
|---|---|
| `notify` (default) | A notification says *Notoir &lt;version&gt; is available* and offers **Download**. |
| `download` | The installer is downloaded in the background. |
| `off` | No automatic checks. |

When the download is verified, a notification offers **Install on Quit**. Nothing is installed until you choose it: Notoir saves your session and recovery data,
quits, and opens the installer (or replaces the AppImage). Every update is verified against a signed manifest and its
SHA-256 checksum before anything is offered. Distribution packages (`.deb`) are left to your package manager.

> Updates only work in builds that were published with signing configured. Otherwise **Check for Updates…** reports
> that updates are unavailable.
