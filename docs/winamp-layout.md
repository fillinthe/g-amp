# Winamp 2 window layout (pixels, at 1× size)

The positions every classic skin is drawn for. Sprite rectangles are in
`crates/gamp-skin/src/sprites.json`.

## Main window (275×116)
- Title bar 0,0 275×14. Buttons 9×9 at y=3: options x=6, minimize 244, shade 254, close 264.
- Clutter bar 10,22 8×43 (O y=3 h8, A 11, I 18, D 25 h8, V 33; h7 otherwise).
- Play/pause indicator 26,28 9×9; work indicator 24,28 3×9.
- Time 39,26: minus at -1,6 (5×1); digits 9×13 at x+9, +21, +39, +51.
- Visualizer 24,43 76×16 (shade mode: 79,5).
- Marquee (TEXT.BMP font) 111,27 154×6.
- kbps 111,43 15×6; kHz 156,43 10×6; mono/stereo 212,41 (mono 27×12, stereo at +27 29×12).
- Volume 107,57 68×13 (thumb 14×11 at y+1); balance 177,57 38×13.
- EQ / PL buttons 219,58 23×12 each.
- Position bar 16,72 248×10 (thumb 29×10).
- Transport 23×18 at y=88: previous 16, play 39, pause 62, stop 85, next 108 (22 wide).
- Eject 136,89 22×16. Shuffle 164,89 47×15; repeat 210,89 28×15. About 253,91 13×15.
- Shade (275×14): previous 169,2 7×10; play 176 10; pause 186 9; stop 195 9; next 204 10; eject 215 10; position 226,4 17×7; mini time 127,4.

## Equalizer (275×116)
- Title bar 0,0 275×14; shade 254,3 and close 264,3 (9×9).
- ON 14,18 26×12; AUTO 40,18 32×12; graph 86,17 113×19; presets 217,18 44×12.
- Sliders 14×63 at y=38: preamp x=21; bands at x=78 + 18·n (60 Hz … 16 kHz).
- Slider background frames: 28 frames of 14×63 in EQ_SLIDER_BACKGROUND, 14 per row, x step 15, row step 65. Thumb 11×11.
- Shade: volume 61,4 97×6; balance 164,4 43×6.

## Playlist (resizable; 275×116 minimum, grows in 25×29 steps)
- Top 20 high: left corner 25, title 100 (centred), tiles, right corner 25. Close at right-2,3; shade at right-12,3.
- Sides: left tile 12 wide, right tile 20 wide (scrollbar handle 8×18 inside), 29 high tiles.
- Bottom 38 high: left corner 125 (ADD 14, REM 43, SEL 72, MISC 101; 22×18 at bottom 12), right corner 150 (LIST at right 22; running time 7,10; transport 10×10 from 3,22; mini time 66,23), visualizer 75 wide (only when wide enough).
- Track rows 13 high, 9px font, from the skin's PLEDIT.TXT colours.

## Volume, balance, position
- Volume: 28 background frames of 68×15 (pick floor(v·27)); balance: 28 frames, by |balance|.
