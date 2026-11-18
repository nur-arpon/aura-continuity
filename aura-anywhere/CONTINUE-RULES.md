# Zero-tap continue: the rules (item 19, 2026-10-21)

No tap at all. Aura guesses, acts, and shows a 2-second **Undo / Pick another** chip. Every rule
below has a unit test with a recorded scenario: phone side `unlock-stage1/phone/.../zerotap/ContinueRules.kt`
(tests `ContinueRulesTest.kt`), PC side `handoff/src-tauri/src/core/zerotap.rs` (tests in the same file),
plus the PC's "what was I doing" pick in `core/pick.rs` and `core/recent.rs`.

## When it fires
| Direction | Trigger (event, no polling) | Needs |
|---|---|---|
| PC -> phone | A supported app comes to the front on the phone: accessibility window event, or (without it) Usage access looked at for 20 s after the screen is unlocked | the PC used a matching item in the last **2 min** |
| Phone -> PC | Brave / Chrome / Edge, WhatsApp Desktop or Claude Desktop comes to the front on the PC (WinEvent foreground hook) | the phone used a matching item in the last **2 min** |

"Used" = playing now, or (paused / a page / a chat) touched within 2 min. On the PC "touched" is the last
keyboard / mouse input (`idle_ms` in the LAST_ACTIVITY answer), so a tab left an hour ago never fires.

## Rules
| # | Rule | Kotlin test | Rust test |
|---|---|---|---|
| 1 | Several things playing: the one producing sound and most recently started wins; the others are untouched | `rule_several_playing_audible_and_newest_wins` | `rule_pc_source_playing_audible_newest_else_last_touched`, `pick::playing_video_beats_paused_video` |
| 2 | Nothing playing, several paused: the one last interacted with wins | `rule_nothing_playing_last_interacted_wins` | same |
| 3 | A YouTube home / feed / search page never wins and is never "playing" | `rule_home_page_never_wins` | `pick::home_page_never_outranks_a_real_page`, `recent::home_page_is_never_playing_and_goes_last`, `rule_not_twice_and_feed_pages_never` |
| 4 | Only within 2 minutes of use | `rule_two_minute_window` | `rule_window_two_minutes` |
| 5 | Exact second: position read at request time (Windows media session on the PC, MediaSession on the phone), extrapolated while playing; clock skew removed with `sent_ms` | `rule_exact_second_extrapolated_while_playing`, `rule_clock_skew_removed_by_sent_ms` | `rule_exact_second_extrapolated_while_playing`, `rule_clock_skew_removed_with_sent_ms`, `recent::media_session_position_replaces_stale_slider` |
| 6 | The owner opened the app and started something else himself within 3 s: hands off | `rule_owner_started_another_video_within_3s` | `rule_owner_started_something_else` |
| 7 | Both devices playing different things: do not hijack either | `rule_both_playing_different_things_hands_off` | `rule_both_playing_different_things` |
| 8 | Ad playing: wait for it to end (re-checked every second, up to 60 s); clips shorter than 30 s are skipped | `rule_ads_wait_short_clips_skip` | `rule_ads_wait_and_short_clips_skip` |
| 9 | Shorts open as Shorts; live streams at the live edge (no t=); playlists keep list= and index= | `rule_shorts_live_playlists` | `rule_shorts_live_playlists` |
| 10 | Same video already open on the target: focus / seek it, no duplicate | `rule_same_video_already_open_just_seek` | `rule_same_item_open_focus_and_seek_no_duplicate` (UIA tab select + media-session seek) |
| 11 | PC: continue INTO the browser + profile where the item last was; brand-new = the browser used most recently (default Brave) | - | `rule_browser_profile_where_it_last_was_else_most_recent` |
| 12 | Phone: several YouTube apps = the one used last for that kind (music -> YT Music build); default the Morphe build | `rule_several_youtube_apps_last_used_default_morphe` | - |
| 13 | Private / incognito: never read, never shared (not in History or session files; a private item is dropped) | `rule_private_never_shared` | (incognito never reaches History / Session files) |
| 14 | After continuing, the source device: pause (default) / mute / leave playing. A browser tab cannot be muted alone from outside, so "mute" pauses on the PC; Android has no per-app mute for another app, so it pauses there too | `rule_after_continue_setting` | `recent::pause_target_matches_title_or_the_only_playing_one` |
| 15 | Never twice for the same report (until it is played again on the source) | (`done` key, 10 min) | `rule_not_twice_and_feed_pages_never` |

## App map (both ways)
| App | PC -> phone | Phone -> PC |
|---|---|---|
| YouTube (Morphe) <-> Brave | the Brave URL at the exact second -> YouTube app (rule 12) | MediaSession title/channel/position; the id from a link the PC sent / a share / a one-time lookup (YouTube search page + oEmbed confidence check: exact title and channel; unsure = nothing) |
| Web pages: Samsung Internet <-> Brave | the URL -> the browser that was opened | the address bar of Samsung Internet / Brave / Chrome (accessibility) -> Brave |
| Facebook app <-> facebook.com | the URL -> Facebook app (app link) | the app shows no link: facebook.com home (partial) |
| Messenger <-> messenger.com | messenger.com/t/<id> -> https://m.me/<id> in Messenger | chat name -> messenger.com/t/<id> by title in History when the web tab had that title, else messenger.com (partial) |
| WhatsApp <-> WhatsApp Desktop / Web | WhatsApp Web has no chat link: opens WhatsApp (partial) | chat name -> number from the phone's contacts -> `whatsapp://send?phone=` (Desktop) or web.whatsapp.com/send (browser); groups: no number, skipped |
| Claude app <-> claude.ai / Claude Desktop | claude.ai/chat/<id> -> the Claude app (app link) | conversation title (accessibility) -> the PC finds claude.ai/chat/<id> by that title in History, else claude.ai/recents |
| Spotify | not automated: Spotify Connect needs the Web API (developer account). Use Spotify's own device picker | the phone reports it (state only); not opened on the PC |

## Privacy
Accessibility service limited to the packages above (`res/xml/aura_accessibility.xml`), reads window titles,
content descriptions, view ids and a browser's address bar only; never typed text; nothing stored or logged; the
one title / link goes to the owner's own PC over Aura Link (end-to-end encrypted). The only action is Back on Undo.
Play Store (later, customers): an app using AccessibilityService for a non-accessibility purpose must fill the
Permissions Declaration (AccessibilityService API) and show a prominent disclosure + consent first
(`ZeroTapSetupActivity` is that screen).

## Settings
Phone: Settings > Zero-tap continue (on by default; PC to phone; Phone to PC; Pause PC / Mute / Leave playing; Set up =
explanation screen -> Accessibility / Usage access / Contacts). PC: `%LOCALAPPDATA%\ContinuityHandoff\settings.json`
`zeroTap` (default true), `afterContinue` ("pause").
