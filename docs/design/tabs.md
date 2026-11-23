# Tabs the app draws

Status: steps 1 to 4 of §8 landed on 23 Nov 2026 (look A, Alex's pick,
with the focused pane's rail kept), but for VoiceOver and the
measurements. Asked for by Alex on 23 Nov 2026, from two screenshots of
0.3.0.

Landed: `tab_host.rs` (the window's content view and delegate, a
`WindowView` a tab), `tab_bar.rs` (the bar, its widths and what a tab
says), `closeTab:`, `Closing::Tab`, the dump's `tab_bar`. Different from
§5 as written: a tab behind is moved out of the window, not hidden (a
hidden view's display link stops); tabs narrower than 90 pt squeeze
instead of scrolling; a tab dragged along the bar swaps places as it
crosses the others, without sliding, and one dragged off the bar shows
nothing until it is let go (then it is a window there, or a tab of the
window whose title row it is over). Nobody had seen it drawn when it
landed: CI's smoke tests check what it holds, not how it looks.

This reverses a goal: DESIGN.md says "no tab bar of our own (tabs are
macOS's native window tabs)", kept for memory (roadmap, E.1: about
2.6 MB a tab for a window, nothing for a bar). §7 says what that costs
now.

## 1. The problem

What the screenshots show, with macOS's tabs on our transparent title
bar:

1. The selected tab is a light grey slab with white text on a dark
   theme: the loudest and least readable thing in the window. It is the
   system's drawing; nothing public changes it.
2. Tabs take a row of their own under the title bar (the top inset goes
   from 28 pt to 53 pt) and each is as wide as the window allows: two
   tabs are two 700 pt slabs.
3. The tab says `waiting · ~/Source/github.com/0xdeafcafe/h2r —
   Terminal Decline`: the app's name in every tab (macOS adds it), the
   whole path, and the status, which a split's pane strip repeats
   straight underneath.
4. A split's dividing line runs up through the tabs and the title bar
   to the top of the window (splits.md §3: "the line itself runs the
   pane's full height", written before there was anything up there).
5. The two pane strips of a split don't match: the focused one is
   brighter, with a rail in the cursor colour across its top, which next
   to the tab row reads as a third kind of bar.

## 2. Decisions at a glance

| Question | Decision | Why |
|---|---|---|
| Who draws tabs | the app, in the theme's colours | 1–3 above are the system's and can't be changed |
| What a tab is | a `WindowView` in a window that holds several, one shown; no longer an `NSWindow` | a bar of ours over system tabs isn't possible: the system's bar can't be hidden while a window has tabs |
| Where the bar is | §3, Alex's pick | |
| A tab's text | the pane strip's text for its focused pane, shortened: the folder's last two parts, or the program's title; never the app's name | one rule for tabs and strips |
| A tab's status | a dot before the text: running (dim), waiting for you (accent), finished in the background (accent ring), failed (red); the word only in the tooltip | "waiting ·" in words is most of what made titles long |
| Width | equal shares of the row, 90 pt to 220 pt each; past that the row scrolls and a `⌄` menu lists every tab | Safari's and Ghostty's; a tab is never 700 pt |
| One tab | no bar: the title row shows the title, as now | nothing to pick between |
| The split line | starts under the bar | 4 above |
| Pane strips | one band for every pane (5%); the focused one keeps its rail in the cursor colour and its brighter text | 5 above; the rail was Alex's pick on 5 Nov and stays unless he says otherwise |
| System tab settings | "Prefer tabs" in System Settings still decides whether a folder dropped on the Dock opens a tab; Merge All Windows and Move Tab to New Window are ours | same menu items, same shortcuts |

## 3. The look

Three candidates, each at the same window width, two tabs, the second
split in two. `●` is the status dot.

**A. In the title row.** Tabs sit beside the window's buttons in the
28 pt title bar. No second row at all: 25 pt (a line and a half of
text) back compared with today.

```
┌──────────────────────────────────────────────────────────────────────┐
│ ● ● ●  ┃ ● h2r · cargo run ┃  ○ rush · Cross App Integr…   +         │
├──────────────────────────────────┬───────────────────────────────────┤
│ cargo run --release -p cli · 39m │ ~/Source/…/h2r                    │
│ present 102700: last 100 at 53.5 │ ❯ █                               │
```

The window is dragged by the empty part of the row and by any tab's
text (a drag that leaves the row moves the tab instead). With many tabs
there is little empty row left to drag.

**B. A slim row of its own.** The title row stays empty (the window's
buttons, and room to drag), and a 24 pt row under it holds the tabs,
flat, in the theme's colours.

```
┌──────────────────────────────────────────────────────────────────────┐
│ ● ● ●                                                                │
│ ┃ ● h2r · cargo run       ┃ ○ rush · Cross App Integr…  │ +          │
├──────────────────────────────────┬───────────────────────────────────┤
│ cargo run --release -p cli · 39m │ ~/Source/…/h2r                    │
```

As tall as today (52 pt against 53). Fixes the look, gives nothing
back.

**C. Tabs down the left.** A 180 pt column, one line a tab, with room
for the whole title and the running time. Hidden with one tab; Cmd+B
folds it to dots.

```
┌──────────────────────────────────────────────────────────────────────┐
│ ● ● ●             │                                                  │
│ ┃● h2r      39m   ├───────────────────────┬──────────────────────────┤
│  ○ rush · Cross…  │ cargo run --release … │ ~/Source/…/h2r           │
│  +                │ present 102700: last  │ ❯ █                      │
```

Good with ten tabs, costs 22 columns of every line with two.

The selected tab in all three: the terminal's own background (it joins
the pane below), text in the foreground colour, a 2 pt bar in the
cursor colour at its leading edge. The others: the bar's band (the
background tinted 4% towards the foreground, as pane strips are), text
half way to the background. Hovering a tab shows its close button in
place of the dot.

Recommended: **A**. It is the only one that gives rows back, and it is
what the transparent title bar was for.

## 4. What a tab does

- Click selects. Cmd+1…9, Cmd+Shift+[ and ], Ctrl+Tab as now (the same
  actions in `term-keys`, now ours instead of AppKit's selectors).
- Drag along the bar reorders, the others sliding aside. Dragged off
  the bar it becomes a window where it is dropped; dropped on another
  window's bar it joins that window. (A drag between windows moves the
  `WindowView` and its sessions: nothing is restarted.)
- Double-click renames (as a pane strip does). Middle-click and the
  hover button close, asking as Close Tab does.
- Right-click: Close Tab, Close Other Tabs, Move Tab to New Window,
  Rename.
- `+` at the end of the bar, and a double-click on the empty bar: new
  tab. With A the double-click zooms the window instead, as a title bar
  does (the `+` is enough).
- VoiceOver: the bar is a tab group (`NSAccessibilityTabGroupRole`),
  each tab a radio button with its full title and status.
- Full screen: the bar slides away with the title row, and pane strips
  show, as now.

## 5. How

- `TabHost`, the window's content view and its delegate: the bar
  (`TabBar`, plain boxes and text fields as `pane_strip.rs` is; no
  `drawRect`) and the tabs' `WindowView`s as subviews, all but the
  selected one hidden. What `WindowView` asks its window for (key,
  close, title, full screen) it asks the host for.
- `App::windows` becomes windows with their tabs:
  `Vec<(NSWindow, TabHost)>`; `VIEWS` and view ids stay, so
  notifications, images, the hand-off and the agent API still name a
  tab by its view's id.
- `Place::TabOf(window)` adds a view to that window's host instead of
  making a window. `visible()` is "selected in its host, and the window
  is"; a hidden tab stops drawing and gives up its drawables as a
  background tab does today.
- The top inset is ours: the title row (28 pt) with A, plus the bar's
  row with B, 0 in full screen. `contentLayoutRect` is still observed
  for the title row.
- Closing: `windowShouldClose` asks for every tab (today's
  `close_all_tabs`); Close Tab asks for one and removes its view; the
  last tab closes the window. `Closing::Tab` joins `Pane` and `Window`
  in `term-activity` so the sheet says "Close this tab?".
- Saving: `SavedWindow` stays one entry a tab with `tab_group` and
  `selected`, so older and newer builds read each other's lists.
- The quick window has no tabs, as now. Settings is a tab, as now.
- System tabbing is turned off for our windows
  (`NSWindowTabbingMode::Disallowed`), so the system never draws its
  bar.

## 6. Tests

- `TabBar` layout as a pure function (widths for n tabs in w points,
  where a drag at x drops, what scrolls): table tests.
- Titles: the shortening rule, table tests.
- Smoke (CI): the nine tab tests stay, reading `tabs` / `tab` from the
  dump, now filled from the host; new ones for reorder, move to a new
  window, close others, and the inset with one tab and with two.

## 7. Cost

- Memory: a tab is no longer a window, so it should be cheaper, not
  dearer (the 2.6 MB was the window). To be measured with `footprint`,
  4 tabs × 4 panes, before and after; the bar itself is a few views.
- Lost: the system's tab overview (View > Show All Tabs) and tabs
  following the system's accent. Mission Control shows a window, not
  its tabs, as now.
- Size: `app.rs` and `view.rs` each assume one view a window in about
  forty places (list in the survey of 23 Nov).

## 8. Plan

1. Pane strips share one band and the split line starts under the top
   inset (done, 23 Nov 2026). Titles change with the bar, in step 3:
   until then the system's tab shows the window's title.
2. `TabHost` with one tab: no visible change, every smoke test passes.
3. Several tabs in a host, the bar (look per §3), select, close, new,
   save and restore.
4. Reorder, drag out and in, the menu, rename, VoiceOver.
5. Measurements, DESIGN.md ("Windows and tabs", the goal).
