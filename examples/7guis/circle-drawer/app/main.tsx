// 7GUIs task 6, Circle Drawer (https://eugenkiss.github.io/7guis/tasks#circle).
//
// Click on an empty part of the canvas to draw a circle there. The circle
// under the pointer is selected and filled. "Adjust diameter" opens a sheet
// with a slider that changes the selected circle at once; closing the sheet
// is one undo step. Undo and Redo go back and forward.
//
// Differences from the task (docs/GAPS.md "7GUIs task 6"): there is no
// right-click context menu, so "Adjust diameter" is a button; and a Dialog
// cannot hold a slider, so the slider is in a Sheet.
import { Math } from "plinth:core";
import { app, signal, Screen, Section, Group, Button, Canvas, Sheet, Slider, Text, rect, circle, strokeCircle, Shape } from "plinth:ui";
import {
  VIEW_W,
  VIEW_H,
  MIN_DIAMETER,
  MAX_DIAMETER,
  circles,
  selected,
  selectedId,
  canUndo,
  canRedo,
  click,
  hover,
  beginAdjust,
  setDiameter,
  endAdjust,
  undo,
  redo,
} from "./model";

function CircleDrawer() {
  const adjusting = signal(false);
  const diameter = signal(0);

  const shapes = (): Shape[] => {
    const out: Shape[] = [rect(0, 0, VIEW_W, VIEW_H, "surface.alt")];
    for (const c of circles()) {
      // The selected circle is filled; every circle has an outline.
      if (c.id === selectedId()) {
        out.push(circle(c.x, c.y, c.d / 2, "text.muted"));
      }
      out.push(strokeCircle(c.x, c.y, c.d / 2, "text", 1.5));
    }
    return out;
  };

  const openAdjust = () => {
    const s = selected();
    if (s === null) {
      return;
    }
    beginAdjust();
    diameter.set(s.d);
    adjusting.set(true);
  };
  const closeAdjust = () => {
    adjusting.set(false);
    endAdjust();
  };

  return (
    <Screen title="Circle Drawer">
      <Section>
        <Group axis="row">
          <Button label="Undo" disabled={!canUndo()} onPress={undo} />
          <Button label="Redo" disabled={!canRedo()} onPress={redo} />
          <Button label="Adjust diameter" disabled={selected() === null} onPress={openAdjust} />
        </Group>
        <Canvas
          label={`Drawing with ${circles().length} circles: click to add a circle`}
          viewWidth={VIEW_W}
          viewHeight={VIEW_H}
          shapes={shapes()}
          onPointerDown={click}
          onPointerMove={hover}
        />
        <Text>{selected() === null ? "No circle selected" : `Selected: diameter ${Math.round(selected()?.d ?? 0)}`}</Text>
      </Section>
      <Sheet open={adjusting} title="Adjust diameter" onClose={closeAdjust}>
        <Slider
          label="Diameter"
          value={diameter()}
          min={MIN_DIAMETER}
          max={MAX_DIAMETER}
          step={1}
          onChange={(d) => {
            diameter.set(d);
            setDiameter(d);
          }}
        />
        <Button label="Done" role="primary" onPress={closeAdjust} />
      </Sheet>
    </Screen>
  );
}

export default app({
  accent: "indigo",
  screens: {
    drawer: { title: "Circle Drawer", icon: "edit", component: CircleDrawer },
  },
});
