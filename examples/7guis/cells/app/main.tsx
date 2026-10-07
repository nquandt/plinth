// 7GUIs task 7, Cells (https://eugenkiss.github.io/7guis/tasks#cells).
//
// A sheet of 26 x 100 cells. Press a cell to select it; its content shows
// in the formula field. Type a number, a text or a formula ("=B2*2",
// "=sum(B2:B3)") and press Enter. A cell shows its value; the cells that
// depend on it update. The rules are in formula.ts and sheet.ts.
//
// Differences from the task (docs/GAPS.md "7GUIs task 7"): a cell is edited
// in a formula field above the sheet, not in place.
import { app, signal, computed, Screen, Section, Box, Span, Pressable, Scroll, TextField, Button } from "plinth:ui";
import { COLS, ROWS, cellName } from "./formula";
import { formulas, values, display, sample } from "./sheet";

const LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const COL_IDS: number[] = [];
for (let c = 0; c < COLS; c++) {
  COL_IDS.push(c);
}
const ROW_IDS: number[] = [];
for (let r = 0; r < ROWS; r++) {
  ROW_IDS.push(r);
}
/** The width of a cell and of the row numbers, in spacing units. */
const CELL_W = 20;
const HEAD_W = 8;
/** The height of a row, so that an empty row is as high as a full one. */
const CELL_H = 8;

sample();

function Cells() {
  const selected = signal(0);
  const draft = signal("");

  const select = (cell: number) => {
    selected.set(cell);
    draft.set(formulas[cell]());
  };
  const commit = () => {
    formulas[selected()].set(draft());
  };
  const info = computed(() => {
    const v = values[selected()]();
    return v.kind === "error" ? `${cellName(selected())}: ${v.text}` : `${cellName(selected())} = ${display(v)}`;
  });

  return (
    <Screen title="Cells">
      <Section>
        <Box direction="row" gap={2} align="end">
          <Box grow={1}>
            <TextField label={`Formula of ${cellName(selected())}`} value={draft} onSubmit={commit} />
          </Box>
          <Button label="Set" role="primary" onPress={commit} />
        </Box>
        <Span fg="text.muted">{info()}</Span>
        <Scroll label="Sheet" direction="row">
          <Box>
            <Box direction="row" bg="surface.alt">
              <Box width={HEAD_W} />
              {COL_IDS.map((c) => (
                <Box width={CELL_W} paddingX={1} align="center">
                  <Span weight="semibold" size="sm">{LETTERS.charAt(c)}</Span>
                </Box>
              ))}
            </Box>
            <Scroll label="Rows" maxHeight={110}>
              {ROW_IDS.map((r) => (
                <Box direction="row">
                  <Box width={HEAD_W} height={CELL_H} paddingX={1} bg="surface.alt" justify="center">
                    <Span weight="semibold" size="sm">{`${r}`}</Span>
                  </Box>
                  {COL_IDS.map((c) => (
                    <Pressable
                      label={cellName(r * COLS + c)}
                      role="button"
                      width={CELL_W}
                      height={CELL_H}
                      paddingX={1}
                      justify="center"
                      border="border"
                      bg={selected() === r * COLS + c ? "selected" : "surface"}
                      onPress={() => select(r * COLS + c)}
                    >
                      <Span size="sm" lines={1}>{display(values[r * COLS + c]())}</Span>
                    </Pressable>
                  ))}
                </Box>
              ))}
            </Scroll>
          </Box>
        </Scroll>
      </Section>
    </Screen>
  );
}

export default app({
  accent: "indigo",
  screens: {
    cells: { title: "Cells", icon: "list", component: Cells },
  },
});
