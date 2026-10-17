2010 Rust Rewrite Mashup - MW2, Skate 3 and Minecraft in one Rust game
======================================================================

No game files are included. You need your own:
  - Call of Duty: Modern Warfare 2 (2009, PC, with multiplayer - Steam works)
  - Optional, for skating: Skate 3 (Xbox 360), extracted - its default.xex
    with the data folder beside it. ISO FILES DO NOT WORK; extract the ISO
    first (e.g. with extract-xiso) and select the default.xex inside.

Minecraft needs nothing from you. The first time the game starts it
downloads Minecraft 26.3's files (about 125 MB) straight from Mojang's
official servers, like the Minecraft launcher does, into
iw4l-artifacts\minecraft-26.3. After that it plays offline.

First launch
  1. Keep this folder somewhere you can write to (not Program Files).
  2. Double-click iw4l.exe.
  3. Confirm your MW2 folder, or select it (the folder with iw4mp.exe and zone).
  4. It asks whether you have Skate 3. Choose Yes and select your
     default.xex (it takes a few seconds to copy out what skating needs),
     or No to play without skating.
  5. The game opens at the menu.

Playing the Minecraft world
  In Create Game, pick the MINECRAFT tab in the map list. The map is called
  "overworld". Or double-click "Minecraft World.bat" to go straight in.

In game
  J           toggle skating (needs a controller and Skate 3)
  E           Minecraft inventory
  1-9, wheel  hotbar slots
  Q           drop the selected item
  `           console (the key under Esc): map minecraft:overworld, skate on/off

Controller
  Plug one in and play. Menus: D-pad or left stick, A select, B back.
  Options > Controller: button layout, stick layout, sensitivity, invert,
    aim assist, vibration and deadzones.
  Options > Controls: every action shows its key and its controller button.
    Select one and press a key or a controller button to rebind it.
  Click both sticks in    toggle skating
  On the Minecraft map:   RT mine, LT place, D-pad left/right hotbar,
                          Y swaps the first two hotbar slots

Start over
  Delete the .env file next to iw4l.exe and run it again.
  The Skate 3 data it extracted is in the skate-data folder.

Source: https://github.com/NoahSurprenant/2010-rust-rewrite-mashup
(a fork of https://github.com/chasmlol/2010-rust-rewrite-mashup)
Unofficial fan project. Not affiliated with Activision, Infinity Ward, EA or
Mojang. Minecraft is a trademark of Mojang Studios.
