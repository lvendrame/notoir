# Updates and file-manager integration

Choose **Help → Add “Open with Notoir”…** to open selected files and folders from your file manager.
Files open as tabs; folders open as workspaces. Default applications stay unchanged. Use **Help → Remove
“Open with Notoir”** to remove the integration.

- macOS: in Finder, select files or folders and choose **Services → Open with Notoir**. macOS may also show
  the service under Quick Actions. Finder’s **Open With → Notoir** opens supported document types.
- Windows: right-click files, folders, or a folder background. On Windows 11 choose **Show more options**.
- Linux: Nautilus’s **Scripts** menu and Dolphin’s **Actions** menu are supported. Other file managers are
  not supported by this integration. Restart the file manager if a newly installed entry is not visible.

If you move Notoir, remove and add the integration again to refresh its application path.

## Checking for updates

**Help → Check for Updates…** reports your current version and checks for a newer stable release.
Choose **Download Update** to download and verify it, then **Quit and Install…** when ready. **Later**
keeps the download ready while this session remains open. Unsaved files are prompted before installation;
cancelling keeps Notoir open. Automatic checks still follow the `update.mode` setting.

On macOS, quitting opens the disk image: drag Notoir into Applications to finish. On Windows, complete
the installer. An AppImage is replaced after quitting; launch it again. Distribution packages use their
package manager. Unsigned or damaged updates are never installed. A build without update signing
configuration explains that updates are unavailable.

## Release signing setup

Maintainers must configure the repository secret `MINISIGN_SECRET_KEY` with a passwordless minisign
secret key and the repository variable `NOTOIR_UPDATE_PUBKEY` with its matching base64 public-key line.
Keep the private key out of source control. `NOTOIR_UPDATE_URL` optionally overrides the default latest
GitHub release manifest URL. Tag releases and manually published releases fail early if either key is
missing. The workflow verifies its signed manifest before publishing; non-publishing packaging runs
remain available without signing credentials. The latest published release must include a signed manifest. Builds without an embedded public key require
a manual installation of a signing-enabled release before in-app updates can work.
