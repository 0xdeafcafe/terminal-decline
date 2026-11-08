window.BENCHMARK_DATA = {
  "lastUpdate": 1794112897462,
  "repoUrl": "https://github.com/0xdeafcafe/terminal-decline",
  "entries": {
    "Scrollback bytes per line": [
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "145de74b46b17d1ac209665b79667136111f8560",
          "message": "feat(app): restart into Terminal Decline.app once the old bundle is gone\n\nA dogfood build found its bundle again from its executable when the\nexecutable changed. Once the bundle is removed rather than replaced, as\nthe dogfood script will do with Decline.app when the app is renamed,\nthat fails and the build quietly stays running with its windows.\n\nKeep the bundle path from startup, and when it is gone, restart into\n\"Terminal Decline.app\" in the same folder with the saved windows. Land\nthis before the rename so the running copy carries its windows over.\n\nCo-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-10-29T03:56:59Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/145de74b46b17d1ac209665b79667136111f8560"
        },
        "date": 1793247584232,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "scrollback CJK",
            "value": 90,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback compiler error",
            "value": 81,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback full-width background",
            "value": 183,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback ls --color",
            "value": 93,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback plain 40 chars",
            "value": 52,
            "unit": "bytes/line"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "cc0828b1d7565ebdcdf9f09ca8ee17f48c97199f",
          "message": "test(app): list iTerm2 and Sixel images in the dump smoke test\n\nWith images = true, an iTerm2 image and a Sixel image printed by the\nprogram must both be counted and ready in the dump's images section,\nwith some text drawn too. Not run locally: CI runs the smoke tests.\n\nCo-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>",
          "timestamp": "2026-10-29T19:23:49Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/cc0828b1d7565ebdcdf9f09ca8ee17f48c97199f"
        },
        "date": 1793335297690,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "scrollback CJK",
            "value": 69,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback compiler error",
            "value": 67,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback emoji-heavy",
            "value": 81,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback full-width background",
            "value": 30,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback gradient 160 cells",
            "value": 2019,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback ls --color",
            "value": 61,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback placeholder image row",
            "value": 57,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback plain 40 chars",
            "value": 44,
            "unit": "bytes/line"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "f72c550128f9a3e7b7089988b6723b0cfa047fba",
          "message": "perf(render): let a hidden tab's every shown frame be purged\n\nA hidden tab marked the surfaces of its last two frames purgeable, but\nthe window server doesn't always keep the last one presented: a tab\nbehind others (and a window in the background test mode) may keep an\nolder one, often the window's first frame, which stayed counted in\nfootprint for as long as the window lived, 6.5 MB at 1624x1018.\n\nThe renderer now keeps the surface of each drawable the layer may hold\n(three, newest first) and marks them all purgeable when the window is\nhidden, and again 0.5 s and 3 s later, for a frame presented just\nbefore the hide (a tab switch that drew once more). Surfaces of another\nsize are let go as soon as a frame of the new size is presented, since\nthe layer never hands them out again. Under the test control a hidden\nwindow still draws (at 1x1); it is hidden again after that frame, and\nthe 1x1 frame leaves the full-size surfaces purgeable, so the test\ncontrol no longer undoes the hide.\n\nMeasured: release, background test mode, temp XDG_CONFIG_HOME, Menlo\n13, 100x30, footprint 40 s after the last frame, interleaved with\norigin/main, load 3-19:\n- 4 tabs: 44.72 / 44.61 / 42.89 / 44.88 / 44.88 MB before,\n  38.64 / 38.13 / 40.56 / 38.22 / 37.08 after; the window-sized\n  image (owned, unmapped, 6.47 MB) is gone in every run. The rest of\n  the spread is a 2 MB GPU driver allocation that comes and goes in\n  both.\n- 2 tabs, switched back to the first: 42.16 / 40.16 / 42.59 before,\n  35.78 / 38.69 / 35.69 after.\n- 1 tab: 28.39 / 28.47 / 28.36 before, 26.95 / 26.94 / 27.17 after\n  (the tab bar change before this one).\nA shown tab's first frame is still committed 1-3 ms after the tab is\nselected.\n\nCo-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-10-31T02:59:22Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/f72c550128f9a3e7b7089988b6723b0cfa047fba"
        },
        "date": 1793421506511,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "scrollback CJK",
            "value": 69,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback compiler error",
            "value": 67,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback emoji-heavy",
            "value": 81,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback full-width background",
            "value": 30,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback gradient 160 cells",
            "value": 2019,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback ls --color",
            "value": 61,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback placeholder image row",
            "value": 57,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback plain 40 chars",
            "value": 44,
            "unit": "bytes/line"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "b14aa9593c843d56217f98a6331b004edfea91fc",
          "message": "docs(design): note the glyph thread under R.1\n\nHow fonts share their registry with the glyph thread, which glyphs\nstay synchronous, how the thread wakes the app, why bitmaps are put in\nthe textures only when drawn and unwanted glyphs skipped, the numbers,\nand what is left: cluster and ligature shaping, and the upload's wait\nfor the GPU.\n\nCo-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>",
          "timestamp": "2026-10-31T18:29:36Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/b14aa9593c843d56217f98a6331b004edfea91fc"
        },
        "date": 1793507902067,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "scrollback CJK",
            "value": 69,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback compiler error",
            "value": 67,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback emoji-heavy",
            "value": 81,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback full-width background",
            "value": 30,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback gradient 160 cells",
            "value": 2019,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback ls --color",
            "value": 61,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback placeholder image row",
            "value": 57,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback plain 40 chars",
            "value": 44,
            "unit": "bytes/line"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "81463a2495d70eed38a537c4ada16bcc4a6d2af4",
          "message": "feat(restore): reopen the windows after any quit\n\nQuitting now always saves the window list (places, tabs, panes, full\nscreen) and the next launch reopens it. History still follows\nrestore_scrollback: saved on quit only with \"always\", never while\nSecure Keyboard Entry is on. Before, a plain quit with the default\nsettings forgot every window and the next launch opened one in the\nmiddle of the screen.",
          "timestamp": "2026-11-02T03:20:23Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/81463a2495d70eed38a537c4ada16bcc4a6d2af4"
        },
        "date": 1793594452794,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "scrollback CJK",
            "value": 76,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback compiler error",
            "value": 66,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback emoji-heavy",
            "value": 85,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback full-width background",
            "value": 26,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback gradient 160 cells",
            "value": 2020,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback ls --color",
            "value": 64,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback placeholder image row",
            "value": 60,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback plain 40 chars",
            "value": 49,
            "unit": "bytes/line"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "41fbb01b4db6a47145d117566552277dd563b48d",
          "message": "perf(core): reserve half the last history block's size, not all of it\n\n50ca009 started each open block as big as the last one ended, which\nleft the open block's spare room at half a block on average: scrollback\nbytes per line rose past the nightly limit for CJK (69 to 76, limit 75)\nand plain text (44 to 49). Reserving half keeps it to one reallocation\nper buffer instead of a dozen: CJK 72, plain 46, every kind under its\nlimit. parse_1k/plain, interleaved runs at load 6-10: 7.39 and 7.07 ms\nagainst 7.06 and 7.22 ms with the whole size, within the noise.",
          "timestamp": "2026-11-02T09:32:04Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/41fbb01b4db6a47145d117566552277dd563b48d"
        },
        "date": 1793614381514,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "scrollback CJK",
            "value": 72,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback compiler error",
            "value": 67,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback emoji-heavy",
            "value": 81,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback full-width background",
            "value": 26,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback gradient 160 cells",
            "value": 2020,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback ls --color",
            "value": 61,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback placeholder image row",
            "value": 56,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback plain 40 chars",
            "value": 46,
            "unit": "bytes/line"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "a053e5b334cdd0edc28531d4fe81aad77d28f6cd",
          "message": "fix(complete): no command line above a pane's first prompt\n\nThe faint line marks one command off from the one before it, but it\nwas drawn above the first prompt too whenever that wasn't on the top\nrow: under the \"restored from …\" or welcome line it only underlined\nthat line. The first prompt has no command above it, so it gets none.",
          "timestamp": "2026-11-02T12:26:38Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/a053e5b334cdd0edc28531d4fe81aad77d28f6cd"
        },
        "date": 1793680959319,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "scrollback CJK",
            "value": 72,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback compiler error",
            "value": 67,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback emoji-heavy",
            "value": 81,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback full-width background",
            "value": 26,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback gradient 160 cells",
            "value": 2020,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback ls --color",
            "value": 61,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback placeholder image row",
            "value": 56,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback plain 40 chars",
            "value": 46,
            "unit": "bytes/line"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "dependabot[bot]",
            "username": "dependabot[bot]",
            "email": "49699333+dependabot[bot]@users.noreply.github.com"
          },
          "committer": {
            "name": "GitHub",
            "username": "web-flow",
            "email": "noreply@github.com"
          },
          "id": "394c9c9a20d04261bfee6e3e313c160fa014fe21",
          "message": "build(deps): bump miniz_oxide from 0.8.9 to 0.9.1 (#13)\n\nBumps [miniz_oxide](https://github.com/Frommi/miniz_oxide) from 0.8.9 to 0.9.1.\n- [Changelog](https://github.com/Frommi/miniz_oxide/blob/master/CHANGELOG.md)\n- [Commits](https://github.com/Frommi/miniz_oxide/commits)\n\n---\nupdated-dependencies:\n- dependency-name: miniz_oxide\n  dependency-version: 0.9.1\n  dependency-type: direct:production\n  update-type: version-update:semver-minor\n...\n\nSigned-off-by: dependabot[bot] <support@github.com>\nCo-authored-by: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>",
          "timestamp": "2026-11-04T20:50:02Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/394c9c9a20d04261bfee6e3e313c160fa014fe21"
        },
        "date": 1793853865429,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "scrollback CJK",
            "value": 72,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback compiler error",
            "value": 67,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback emoji-heavy",
            "value": 81,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback full-width background",
            "value": 26,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback gradient 160 cells",
            "value": 2020,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback ls --color",
            "value": 61,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback placeholder image row",
            "value": 56,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback plain 40 chars",
            "value": 46,
            "unit": "bytes/line"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "a4086db1155e9f82785501269214d3fdbadf2f73",
          "message": "docs(update): design What's New (updater.md §12)",
          "timestamp": "2026-11-05T12:13:04Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/a4086db1155e9f82785501269214d3fdbadf2f73"
        },
        "date": 1793940140674,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "scrollback CJK",
            "value": 72,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback compiler error",
            "value": 67,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback emoji-heavy",
            "value": 81,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback full-width background",
            "value": 26,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback gradient 160 cells",
            "value": 2020,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback ls --color",
            "value": 61,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback placeholder image row",
            "value": 56,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback plain 40 chars",
            "value": 46,
            "unit": "bytes/line"
          }
        ]
      },
      {
        "commit": {
          "author": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "committer": {
            "name": "Alexander Forbes-Reed",
            "username": "0xdeafcafe",
            "email": "git@alex.forbes.red"
          },
          "id": "14f1c8e206d5d978b07ae61a8f513f50034438b4",
          "message": "fix: tighten terminal preview typography",
          "timestamp": "2026-11-07T10:52:42Z",
          "url": "https://github.com/0xdeafcafe/terminal-decline/commit/14f1c8e206d5d978b07ae61a8f513f50034438b4"
        },
        "date": 1794112895945,
        "tool": "customSmallerIsBetter",
        "benches": [
          {
            "name": "scrollback CJK",
            "value": 72,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback compiler error",
            "value": 67,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback emoji-heavy",
            "value": 81,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback full-width background",
            "value": 26,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback gradient 160 cells",
            "value": 2020,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback ls --color",
            "value": 61,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback placeholder image row",
            "value": 56,
            "unit": "bytes/line"
          },
          {
            "name": "scrollback plain 40 chars",
            "value": 46,
            "unit": "bytes/line"
          }
        ]
      }
    ]
  }
}