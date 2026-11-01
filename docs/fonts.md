# Fonts and text

Every setting here goes in `~/.config/terminal-decline/config.toml` (or a
pin's own file), is in the Font section of Settings (Cmd+,), and applies
to every open window as soon as the file is saved. A value the app can't
use is a notice in the terminal and is left out; the rest of the file
still loads.

## Picking a font

```sh
terminal-decline fonts            # every installed family; * marks monospaced ones
terminal-decline fonts --mono     # only the monospaced ones
terminal-decline fonts --styles   # each family's faces, with their PostScript names
```

In Settings, Family opens a list of the installed fonts: the monospaced
ones first, then other fonts (proportional fonts work, but their letters
won't line up in the grid). Type to search. Moving through the list
shows the font under the selection in every window at once; Enter keeps
it, Esc puts back the one you had. The bold and italic face rows list the
family's own faces the same way, and Fallback adds fonts from the same
list.

| Setting | Default | Example |
| --- | --- | --- |
| `font_family` | `"Menlo"` | `font_family = "JetBrains Mono"` |
| `font_size` | `13` (points) | `font_size = 14.5` |
| `font_size_step` | `1` | `font_size_step = 2`: Cmd+= and Cmd+- change a window's size by 2 points; Cmd+0 puts it back |
| `font_fallback` | `[]` | `font_fallback = ["Symbols Nerd Font Mono", "Apple Color Emoji"]`: tried in order for characters the family lacks, before the system's own fallback |
| `font_bold` | the family's own | `font_bold = "JetBrains Mono ExtraBold"` |
| `font_italic` | the family's own | `font_italic = "Victor Mono Italic"` |
| `font_bold_italic` | the family's own | `font_bold_italic = "Victor Mono Bold Italic"` |
| `font_synthetic_bold` | `true` | `font_synthetic_bold = false`: a family with no bold face draws bold text regular instead of thickened |
| `font_synthetic_italic` | `true` | `font_synthetic_italic = false`: a family with no italic face draws italic text upright instead of slanted |
| `symbol_map` | none | `symbol_map = { "U+E000-U+F8FF,U+F0000-U+FFFFD" = "Symbols Nerd Font Mono" }`: these code points come from that font first (Nerd Font icons with any family) |
| `emoji_font` | Apple Color Emoji | `emoji_font = "Noto Color Emoji"`: emoji the font has come from it |

Faces and fonts are named by family ("Menlo"), full name ("Menlo Bold
Italic") or PostScript name ("Menlo-BoldItalic"). A name that isn't
installed is a notice naming its setting, and is skipped.

## Ligatures and OpenType features

Ligatures are on wherever the font has them: `->`, `!=` and `===` in
fonts such as JetBrains Mono, Fira Code or Cascadia Code are drawn as one
glyph across their cells, and every cell keeps its own colour, cursor
and selection.

| Setting | Default | Example |
| --- | --- | --- |
| `ligatures` | `true` | `ligatures = false`: turns `calt`, `liga` and `dlig` off, whatever `font_features` says |
| `font_features` | `[]` | `font_features = ["zero", "ss01", "cv05=2", "-calt"]`: `tag` or `+tag` turns a feature on, `-tag` off, `tag=n` picks alternate n; for all four faces |
| `font_features_bold` | `[]` | `font_features_bold = ["-ss01"]`: the bold face only, after `font_features` |
| `font_features_italic` | `[]` | `font_features_italic = ["ss01"]`: a cursive italic in fonts that have one |
| `font_features_bold_italic` | `[]` | `font_features_bold_italic = ["ss01"]` |

## Weight and variable fonts

| Setting | Default | Example |
| --- | --- | --- |
| `font_variations` | `[]` | `font_variations = ["wght=450"]`: variable font axes for the regular face (`wght` weight, `wdth` width, `slnt` slant, `opsz` optical size); fonts without the axis ignore it |
| `font_variations_bold` | `[]` | `font_variations_bold = ["wght=650"]` |
| `font_variations_italic` | `[]` | `font_variations_italic = ["wght=450", "slnt=-8"]` |
| `font_variations_bold_italic` | `[]` | `font_variations_bold_italic = ["wght=650"]` |
| `font_thicken` | `false` | `font_thicken = true`: glyphs drawn a little heavier, as macOS's font smoothing used to; easier to read thin fonts on dark backgrounds |
| `font_thicken_strength` | `255` | `font_thicken_strength = 128`: how much, 0 to 255 (255 is half a point of stroke around each glyph) |

## Cell size and lines

Adjustments take points (`"1.5"`, `-1`, a plain number is points too) or
a percentage (`"10%"`). Positions move by a percentage of the cell
height, thicknesses by a percentage of themselves.

| Setting | Default | Example |
| --- | --- | --- |
| `line_height` | `1` | `line_height = 1.2`: multiplies the cell height; the text stays centred |
| `adjust_cell_height` | `"0"` | `adjust_cell_height = "2"` or `"10%"`: added after `line_height` |
| `cell_width` | `"0"` | `cell_width = "-0.5"` or `"5%"`: wider or narrower cells (Ghostty's `adjust-cell-width`) |
| `adjust_baseline` | `"0"` | `adjust_baseline = "1"`: the text one point higher in its cell; negative lowers it |
| `adjust_underline_position` | `"0"` | `adjust_underline_position = "1"`: underlines a point lower; negative raises them |
| `adjust_underline_thickness` | `"0"` | `adjust_underline_thickness = "50%"` |
| `adjust_strikethrough_position` | `"0"` | `adjust_strikethrough_position = "-1"`: the strikethrough a point higher |
| `adjust_strikethrough_thickness` | `"0"` | `adjust_strikethrough_thickness = "0.5"`: by default it is as thick as the font's underline |
| `adjust_cursor_thickness` | `"0"` | `adjust_cursor_thickness = "100%"`: a bar or underline cursor twice as thick |

Underlines and the strikethrough always stay inside their cell, however
far they are moved.

## Colour and width

| Setting | Default | Example |
| --- | --- | --- |
| `minimum_contrast` | `1` (off) | `minimum_contrast = 3`: text with less contrast against its background than this (WCAG ratio, 1 to 21) is drawn lighter or darker until it has it; 4.5 is WCAG AA |
| `bold_is_bright` | `false` | `bold_is_bright = true`: bold text in the first 8 ANSI colours uses their bright versions, as older terminals did |
| `ambiguous_width` | `1` | `ambiguous_width = 2`: East Asian Ambiguous characters (Ⅰ, ○, ±) take two cells, as CJK terminals draw them |
| `cursor_style` | `"block"` | `cursor_style = "bar"` (or `"underline"`), until a program sets its own |
| `cursor_blink` | `false` | `cursor_blink = true` |

## A full example

```toml
font_family = "JetBrains Mono"
font_size = 14
font_size_step = 1
font_variations = ["wght=420"]
font_features = ["zero", "cv05"]
font_features_italic = ["ss01"]
font_fallback = ["Apple Color Emoji"]
symbol_map = { "U+E000-U+F8FF" = "Symbols Nerd Font Mono" }
line_height = 1.1
adjust_underline_position = "1"
adjust_cursor_thickness = "50%"
font_thicken = true
font_thicken_strength = 160
minimum_contrast = 1.5
```

Not there yet: blinking text (SGR 5 is kept but drawn steady), and
splitting ligatures at the cursor.
