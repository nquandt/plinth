// 7GUIs task 6, Circle Drawer (https://eugenkiss.github.io/7guis/tasks#circle).
//
// Click on an empty part of the canvas to draw a circle there. The circle
// under the pointer is selected and filled. A right-click on a circle (or
// the menu key or Shift+F10 on the focused canvas, for the selected circle)
// opens a menu at the pointer with "Adjust diameter...". It opens a dialog
// with a slider (UI API 1.13: a Dialog holds a small form) that changes the
// selected circle at once; closing the dialog is one undo step. The "Adjust
// diameter" button does the same from the keyboard. Undo and Redo go back
// and forward.
import { Math } from "plinth:core";
import { app, signal, Screen, Section, Group, Button, Canvas, Menu, Action, Dialog, Slider, Text, rect, circle, strokeCircle, Shape } from "plinth:ui";
import {
  VIEW_W,
  VIEW_H,
  MIN_DIAMETER,
  MAX_DIAMETER,
  circles,
  circleAt,
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
  // The context menu of a circle (UI API 1.13).
  const menuOpen = signal(false);

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
  // A right-click selects the circle under the pointer; the menu opens for
  // the selected circle. The menu key gives the center of the canvas, so it
  // opens the menu for the selected circle.
  const contextMenu = (x: number, y: number) => {
    const hit = circleAt(x, y);
    if (hit !== null) {
      selectedId.set(hit.id);
    }
    if (selected() !== null) {
      menuOpen.set(true);
    }
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
          onPointerDown={(x, y) => {
            // A click outside the open menu only closes it.
            if (!menuOpen()) {
              click(x, y);
            }
          }}
          onPointerMove={(x, y) => {
            if (!menuOpen()) {
              hover(x, y);
            }
          }}
          onContextMenu={contextMenu}
        />
        <Menu label="Circle" open={menuOpen} actions={[<Action label="Adjust diameter..." onPress={openAdjust} />]} />
        <Text>{selected() === null ? "No circle selected" : `Selected: diameter ${Math.round(selected()?.d ?? 0)}`}</Text>
      </Section>
      <Dialog
        open={adjusting}
        title="Adjust diameter"
        message={`The circle at (${Math.round(selected()?.x ?? 0)}, ${Math.round(selected()?.y ?? 0)}).`}
        onClose={closeAdjust}
        actions={[<Action label="Done" role="primary" onPress={closeAdjust} />]}
      >
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
      </Dialog>
    </Screen>
  );
}

export default app({
  accent: "indigo",
  screens: {
    drawer: { title: "Circle Drawer", icon: "edit", component: CircleDrawer },
  },
});
