# Screenshots

The repository's README references two images by exact name. Both are captured;
replace a file in place rather than renaming it, or the README stops rendering.

| File              | What to capture                                                                  |
| ----------------- | -------------------------------------------------------------------------------- |
| `llm-advisor.png` | The LLM tab, showing the fit matrix with verdicts.                               |
| `chat.png`        | The chat page mid-conversation, with the context meter and token counts visible. |

Both were taken on Windows at a 1744x1112 window, which is wide enough for the
fit matrix to show all four quantisations without a horizontal scrollbar. A
narrower window drops columns off the right edge, and a screenshot of a cropped
table is worse than none.

## Before you commit one

Screenshots of the Overview, Processes and Ports tabs contain **real process
names, PIDs, open ports and remote addresses**. Git history keeps an image even
after it is replaced, so check what is in frame first. The two files above were
chosen because neither screen shows that data.

Windows: `Win`+`Shift`+`S`, or `Alt`+`PrtScn` for the focused window.
