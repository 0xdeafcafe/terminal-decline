# Generated pin samples

Written by `term-icon` (`slope-pins-2`), for looking at; not part of the
brand asset set. Regenerate with:

```sh
cargo run --release -p term-icon --example samples -- docs/brand/samples-generated
```

`<name>.png` is 512 px; `<name>-32.png` and `<name>-16.png` are drawn at
those Dock sizes, not scaled down.

| Name | FNV-1a 32 | Index | Hue | Initial |
| --- | ---: | ---: | --- | --- |
| Rush | 20943315 | 5 | Teal | R |
| Claude | 3791422743 | 3 | Violet | C |
| htop | 3212443446 | 6 | Indigo | H |
| lazygit | 432441877 | 7 | Periwinkle | L |
| btop | 3363480540 | 0 | Cyan | B |
| nvim | 3182221067 | 7 | Periwinkle | N |
| ssh prod | 108784458 | 8 | Orchid | S |
| yazi | 1583881906 | 6 | Indigo | Y |
| Logs | 3473062022 | 2 | Amber | L |
| Vim | 1964130039 | 9 | Plum | V |
| Kube | 3501902920 | 0 | Cyan | K |
| Docker | 199450061 | 1 | Azure | D |

With ten hues, names share one often (lazygit and nvim, htop and yazi);
the initial still tells them apart. Logs and lazygit share the initial
L but not the hue (Amber, Periwinkle).
