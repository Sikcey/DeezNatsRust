# Themes

In **Settings > Appearance**, use **Export theme** to save the current appearance
or **Import theme** to apply a saved theme immediately. Imported settings persist
across restarts. Existing palette, color and backdrop controls remain editable.

Try [Midnight](themes/Midnight.deeznats-theme.json), or export your own appearance
and edit its JSON. `format` must be `deeznats-theme`, and `version` must be `1`.
Both dark and light color maps are supported. Colors use `#RRGGBB`.

Themes include palette colors, light/dark mode, backdrop mode and tint, blur,
brightness, corner style and player appearance. Files contain appearance data
only: account details, playback settings and local wallpaper paths are excluded.
An image-mode theme uses the wallpaper already selected on the receiving device;
choose one in Appearance if none is selected. Themes never fetch wallpapers.

Import validates the full file before changing the current appearance. Files are
limited to 64 KiB; unsupported versions, malformed colors and out-of-range values
are rejected. Themes do not execute scripts or load plugins.
