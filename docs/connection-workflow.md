# Connections and exceptional setup flows

## Check an affected layer

A source-driven layer shows a short connection note in its selected inspector.
**Connections: …** opens that source and keeps the layer selected. Use
**Repair layer binding** to review a local binding change, then return to
**Editor** and Save explicitly. Merely following the link does not modify the
draft, start a producer or activate lighting.

Recent data, registration and process health are different facts:

- **Connection missing** refers to the last connection registry report.
- **No recent data** does not prove an application stopped or lost focus.
- **Live data delayed** means retained observations are not current.
- **Recent data available** describes real source observations, not process
  health, selected preview appearance or physical keyboard output.
- Process health is unknown for externally managed publishers and when a
  current health report is unavailable. Managed-process summaries are a point-in-
  time check, not proof that a layer's conditions match.

Private preview scenarios never become live connection observations.

## Browse apps first

The Connections overview shows each app's **current reporting sessions or
instances**, a breakdown of their reported states, and **Used by** links to
layers in the current draft. A layer appears once even when several of its
appearances use the app. Opening a layer preserves its draft; it does not enable
lighting.

Counts include only fresh state reports. Removed/expired sessions and retained
history are excluded. **Live data delayed** means current session activity is
unknown, not idle. An event-only publisher can show **Recent activity received**
without establishing session presence.

Open **App details** for current sessions and their last-update ages. Short
session IDs identify reports, not tmux pane numbers or focus; hover for the full
ID. **Setup instructions** explains how the app reports activity. Pi sessions
need the Underglow extension loaded against the same engine; session IDs change
when sessions restart.

## Open technical details deliberately

Within App details, **Advanced details** contains exact IDs, lease timing,
process health, capabilities, event history and per-appearance binding repair.
Externally run Pi sessions normally have unknown process health because
Underglow does not supervise them; that is not itself a reporting error.

The page's **Advanced** section contains **Connection report details** (capture
age and registry revision) and **Connection administration** (capability-manifest
import and producer configuration). Managed health older than ten seconds is
treated as unavailable until refreshed. Executable review, explicit trust and
separate enablement controls remain unchanged. Browsing any of these sections
never starts a producer or enables lighting. Diagnostics remain available from
the header; engine settings remain separate.

## Share without switching

Open the **setup-name switcher**, then **Share setups**. The explicit import/export
window preserves the strict existing setup format and local-file/paste routes.
An imported source role still requires a local connection and an explicit
instance choice, including disabled layers and appearances.

**Store imported new copy (do not switch)** changes only the saved library. It
does not replace the current draft or grant executable trust. Opening the new
copy and activating lighting remain separate actions. Exported setups contain
declarative visuals and bindings, not machine-local credentials or producers.

## Keep legacy behavior or begin a setup

For a legacy profile, the Editor first offers **Keep current behavior** or
**Begin new setup**. Keep exposes the existing whole-profile controls without
changing anything. Begin opens the setup browser; choosing a starter creates a
local editable draft, including while offline. No saved snapshot changes yet.

Saving or opening a new editable setup requires **Confirm setup adoption** before
any replacement action is sent. The confirmation explains that legacy visuals,
notifications and automation rules in the private engine snapshot are replaced;
unsupported legacy behavior is not silently carried across. The original source
file and machine-local Connections, credentials and trust remain unchanged.

**Cancel adoption** retains the local draft and existing lighting. If lighting
is active, the separate **Pause and save/open** confirmation is also required.
Neither adoption, import, editing nor saving automatically resumes lighting.
