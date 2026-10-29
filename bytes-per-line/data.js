window.BENCHMARK_DATA = {
  "lastUpdate": 1793247585237,
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
      }
    ]
  }
}