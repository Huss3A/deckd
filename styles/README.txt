deckd window styles
===================

A style is a TOML file that changes how the deckd window looks and is laid out.

  * Put it here as  name.toml   or as a folder  name\style.toml  (a folder can also hold a font and a background image).
  * Pick it in the window: the gear icon, Settings, Style. Saving the file updates the open window live.
  * "Create a style to edit" in Settings writes a commented template (my-style\style.toml) and selects it.
  * Anything a style leaves out falls back to its base ("dark" or "light"), so a style can be three lines long.
  * A style only affects this window. It never changes what the deck shows.
  * A broken style never breaks the window: deckd keeps the previous look and shows what is wrong.

"Create a style to edit" lists every option with its default value (colors, shapes, fonts, layout, background image).
midnight.toml, paper.toml and terminal.toml are examples. Share a style by sharing its file or folder.
