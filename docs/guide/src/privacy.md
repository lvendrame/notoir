# Notoir Privacy Policy

**Last updated: 8 November 2026**

**Publisher and privacy contact:** Luís Fernando Saquetim Vendrame  
**Email:** [lfsvendrame@gmail.com](mailto:lfsvendrame@gmail.com)

This policy explains how Notoir handles information when you use the desktop application and its official
documentation. It covers local processing, optional connections to other services, and information you
choose to send to the publisher for support.

## Overview

Notoir is a native text editor and workspace application. You do not need a Notoir account to edit files.
Document editing, search and previews are processed on your device. Notoir does not include advertising
or a usage-analytics SDK, and it does not automatically upload your documents to the publisher.

Information can leave your device when you use or configure network features, connect an external client,
install a plugin with network permissions, contact support, or use services supplied by your operating
system or app store. These cases are described below.

## Information processed on your device

Depending on the features you use, Notoir processes and stores:

- The contents of documents you open or create, including unsaved text, scratchpads and recovery copies.
- File names and paths, workspace locations, recent items, bookmarks, and editor/session state.
- Settings, templates, installed plugins, permission choices and client-access records.
- Workspace search indexes and caches, which can contain text from your workspace files.
- Local diagnostic logs and MCP/plugin activity records. These may identify files, operations, errors,
  URLs or other details relevant to troubleshooting.

This information supports editing, searching, restoring work, managing your workspace, and the integrations
you choose. It is stored in your selected files and Notoir's local application directories. See
[data locations](troubleshooting.md#where-notoir-keeps-its-data).

Local processing does not give the publisher access to the files on your computer. Files in a synced
folder or an operating-system backup may also be copied by the service you use for that folder or backup.

## Optional connections and sharing

### Git and synchronization

Git fetch, pull, push, clone and configured synchronization features communicate with the remote servers
you select. A push can transmit repository contents, commit history and Git author information, such as
names and email addresses. Features such as automatic fetch or push can operate automatically after you
enable them. Your Git provider and credential tools handle those connections under their own terms.

### URLs and external links

Opening a document from a URL contacts that website to download the requested content. Opening an external
link may launch your browser or another application. The destination can receive your IP address, the
requested URL and normal connection/request metadata. Information included in a URL is shared with its
destination. The destination's privacy policy applies to its handling of that information.

### Plugins

Plugins are optional and have their own permissions. Depending on the permissions you grant, a plugin
can access documents, workspace files, the clipboard, settings or network services, and may run external
programs. A plugin with relevant access can transmit information to third-party services. Review the
plugin's permissions and privacy information before enabling it. You can disable or uninstall plugins
and revoke permissions in Notoir.

### MCP and AI clients

Notoir's optional MCP server allows authenticated clients to perform actions permitted by their access
settings. The server binds to the local loopback interface. It does not start automatically unless you
enable that behavior.

An authorized client can read, search or modify permitted information. Permissions set to Allow can be
used without a new prompt for every request. An AI client may send information it reads to its own
provider or other services according to that client's configuration and privacy policy. Notoir does not
choose an AI provider for you. You can review permissions, revoke a client or stop the MCP server.

### Software updates

The Microsoft Store edition uses Microsoft Store updates and disables Notoir's standalone updater.
Direct-download editions can contact the configured update service, normally GitHub, for update metadata
and installers. Those requests include network metadata and the updater's application version in its
user-agent header; they do not require uploading your documents. Automatic checks depend on your update
settings, and manually checking or downloading an update also contacts the update service.

## Diagnostics, Microsoft Store and Windows

Notoir writes diagnostic logs locally; it does not include its own automatic log-upload or crash-report
submission service. The standard build rotates diagnostic log files daily and keeps up to seven files.
Plugin output and information associated with an error may contain sensitive details, so review files
before choosing to share them with support.

Microsoft Store and Windows separately process information about distribution, installation, updates and
diagnostics under [Microsoft's Privacy Statement](https://www.microsoft.com/privacy/privacystatement)
and your Microsoft/Windows settings. Store-provided reports can make acquisition, reliability and error
information available to the publisher. Diagnostic reports or crash dumps supplied through these services
may contain device/app details and information from the process at the time of a failure. This processing
is separate from Notoir's local logs and is not governed solely by Notoir's settings.

## Information you send to the publisher

If you email the privacy/support contact, the publisher receives your email address, message, attachments
and information you choose to provide. These are used to respond to your request, investigate reported
problems and maintain Notoir. They are not used for advertising or sold.

Correspondence is handled through Gmail and is therefore also processed by Google's email service under
[Google's Privacy Policy](https://policies.google.com/privacy). Information you post in a public GitHub
issue or Store review is visible according to the service's publication settings and policies.

For personal information received by the publisher, support and troubleshooting rely on the legitimate
interest in providing assistance and maintaining the application. Responding to statutory privacy requests
or retaining records required by law relies on applicable legal obligations. Support correspondence is
retained only for as long as needed for these purposes and applicable legal requirements. You can contact
the publisher to request access, correction or deletion of correspondence.

## Retention and your controls

Saved documents remain where you save them until you or your storage provider remove them. Closing a
document does not necessarily delete scratchpads, recovery copies, indexes, recent-item records or logs.
Local caches and application data can remain until replaced, purged by the application, or deleted by you
or the operating system.

You can:

- Edit or delete your documents, scratchpads and application data.
- Review recovery copies and clear or remove cached/indexed information.
- Disable optional synchronization and network-enabled plugins.
- Review and revoke MCP/client and plugin permissions, and stop the MCP server.
- Change direct-download update settings and Windows/Microsoft Store diagnostic settings.
- Choose whether to email documents or logs to support.

Removing local information does not automatically remove copies already held in Git history, backups,
synced folders or third-party services. Uninstalling Notoir does not delete documents saved elsewhere.
The publisher cannot remotely retrieve or erase information that exists only on your device.

## Storage and security

Notoir uses application permissions and MCP authentication to control supported access. Local files and
application data rely on your operating system's access controls and storage protection; Notoir does not
provide built-in encryption for all documents, recovery copies, indexes or logs. Network protection depends
on the protocol and service you choose, including whether a URL uses HTTPS.

External providers may process information in countries other than your own. Their processing, retention
and international-transfer arrangements are described in their respective privacy policies. This includes
Microsoft services, GitHub, Gmail, Git hosts and any plugin or AI provider you use.

## Documentation website

The official documentation is hosted on GitHub Pages. The desktop application and documentation do not
include advertising trackers. When you visit the website or repository, GitHub processes normal service
and request information under the
[GitHub Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).
Links to other websites are governed by those websites' policies.

## Your privacy rights

Depending on applicable law and the information concerned, you may have rights to request access,
correction, deletion, restriction or portability, or to object to processing. Send requests concerning
information held by the publisher to [lfsvendrame@gmail.com](mailto:lfsvendrame@gmail.com).
You may also complain to your competent data-protection authority.

For information held only on your device, use the local controls described above. For information handled
independently by Microsoft, GitHub, Google or another provider, use that provider's privacy-request process.

## Changes and contact

Changes to this policy will be posted on this page with an updated date. For questions about Notoir's
privacy practices or to make a privacy request, contact **Luís Fernando Saquetim Vendrame** at
**[lfsvendrame@gmail.com](mailto:lfsvendrame@gmail.com)**.
