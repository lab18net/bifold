//! Built-in themes, in the same TOML format as user themes.
//!
//! `terminal`, `nord`, `gruvbox`, `gruvbox-light`, `solarized` and `solarized-light` use the palettes
//! of herdr 0.9.5 (`src/app/state.rs`); `bifold` is this app's own default look.
//! Generated once from those sources — edit by hand from here on.

pub const BUILTIN: &[(&str, &str)] = &[
    (
        "bifold",
        r##"accent = "#60d0c0"
panel_bg = "reset"
active_row_bg = "#1c3a38"
selection_bg = "#2a261a"
surface0 = "#2e3034"
surface1 = "#40444a"
surface_dim = "#262a2e"
overlay0 = "#50565c"
overlay1 = "#646a70"
text = "#dce0e4"
subtext0 = "#70767c"
mauve = "#ba92e8"
green = "#82c882"
yellow = "#e8c87a"
red = "#e87070"
blue = "#7aacff"
teal = "#60d0c0"
peach = "#f0a06e"
"##,
    ),
    (
        "terminal",
        r##"accent = "blue"
panel_bg = "reset"
active_row_bg = "darkgray"
selection_bg = "reset"
surface0 = "reset"
surface1 = "darkgray"
surface_dim = "darkgray"
overlay0 = "gray"
overlay1 = "white"
text = "reset"
subtext0 = "gray"
mauve = "gray"
green = "green"
yellow = "yellow"
red = "lightred"
blue = "blue"
teal = "cyan"
peach = "yellow"
"##,
    ),
    (
        "nord",
        r##"accent = "#88c0d0"
panel_bg = "#2e3440"
active_row_bg = "#434c5e"
selection_bg = "#40505d"
surface0 = "#3b4252"
surface1 = "#434c5e"
surface_dim = "#2e3440"
overlay0 = "#4c566a"
overlay1 = "#646e82"
text = "#eceff4"
subtext0 = "#d8dee9"
mauve = "#b48ead"
green = "#a3be8c"
yellow = "#ebcb8b"
red = "#bf616a"
blue = "#81a1c1"
teal = "#8fbcbb"
peach = "#d08770"
"##,
    ),
    (
        "gruvbox",
        r##"accent = "#d79921"
panel_bg = "#282828"
active_row_bg = "#323130"
selection_bg = "#4b3f27"
surface0 = "#3c3836"
surface1 = "#504945"
surface_dim = "#282828"
overlay0 = "#928374"
overlay1 = "#a89984"
text = "#ebdbb2"
subtext0 = "#d5c4a1"
mauve = "#d3869b"
green = "#b8bb26"
yellow = "#fabd2f"
red = "#fb4934"
blue = "#83a598"
teal = "#8ec07c"
peach = "#fe8019"
"##,
    ),
    (
        "gruvbox-light",
        r##"accent = "#076678"
panel_bg = "#fbf1c7"
active_row_bg = "#f2e5bc"
selection_bg = "#ebdbb2"
surface0 = "#ebdbb2"
surface1 = "#d5c4a1"
surface_dim = "#f2e5bc"
overlay0 = "#928374"
overlay1 = "#7c6f64"
text = "#3c3836"
subtext0 = "#504945"
mauve = "#8f3f71"
green = "#79740e"
yellow = "#b57614"
red = "#9d0006"
blue = "#076678"
teal = "#427b58"
peach = "#af3a03"
"##,
    ),
    (
        "solarized",
        r##"accent = "#268bd2"
panel_bg = "#002b36"
active_row_bg = "#164b57"
selection_bg = "#083e55"
surface0 = "#073642"
surface1 = "#586e75"
surface_dim = "#002b36"
overlay0 = "#586e75"
overlay1 = "#657b83"
text = "#93a1a1"
subtext0 = "#839496"
mauve = "#d33682"
green = "#859900"
yellow = "#b58900"
red = "#dc322f"
blue = "#268bd2"
teal = "#2aa198"
peach = "#cb4b16"
"##,
    ),
    (
        "solarized-light",
        r##"accent = "#268bd2"
panel_bg = "#fdf6e3"
active_row_bg = "#eee8d5"
selection_bg = "#c9dcdf"
surface0 = "#eee8d5"
surface1 = "#93a1a1"
surface_dim = "#eee8d5"
overlay0 = "#93a1a1"
overlay1 = "#586e75"
text = "#657b83"
subtext0 = "#839496"
mauve = "#d33682"
green = "#859900"
yellow = "#b58900"
red = "#dc322f"
blue = "#268bd2"
teal = "#2aa198"
peach = "#cb4b16"
"##,
    ),
];
