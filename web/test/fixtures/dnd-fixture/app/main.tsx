// The drag-and-drop fixture (UI API 1.13, PRODUCTION U6): the same app as
// crates/plinth-shoot/tests/drag_drop.rs. web/test/run-a11y.mjs
// (`checkDragDrop`) drives it in the browser.
import { app, signal, Screen, Text, Button, Box, Scroll, Dialog, Action, List, Row, Slider, TextField } from "plinth:ui";

const items = signal(["Apple", "Banana", "Cherry"]);
const log = signal("none");
const dialog = signal(false);
const size = signal(10);
const name = signal("");

function move(from: number, to: number) {
  const all = items();
  const rest: string[] = [];
  for (let i = 0; i < all.length; i++) {
    if (i !== from) rest.push(all[i]);
  }
  const out: string[] = [];
  for (let i = 0; i < rest.length; i++) {
    if (i === to) out.push(all[from]);
    out.push(rest[i]);
  }
  if (to >= rest.length) out.push(all[from]);
  items.set(out);
  log.set(`reorder ${from} ${to}`);
}

function Home() {
  return (
    <Screen title="Drag">
      <Text>{`log: ${log()}`}</Text>
      <Text>{`order: ${items().join(",")}`}</Text>
      <Text>{`size: ${size()}`}</Text>
      <List items={items()} key={(s) => s} row={(s) => <Row title={s} dragValue={s} />} onReorder={move} />
      <Box label="Basket" width={60} height={20} bg="surface" border="border"
           onDrop={(v, x, y) => log.set(`drop ${v} ${x > 0 && x < 60} ${y > 0 && y < 20}`)} />
      <Scroll label="Strip" direction="row" width={60} height={14} bg="surface.alt">
        <Box width={60} height={10} />
        <Box width={60} height={10} />
        <Box label="Far basket" width={30} height={10} bg="surface" border="border"
             onDrop={(v) => log.set(`far drop ${v}`)} />
      </Scroll>
      <Box label="Card" dragValue="card" width={20} height={8} bg="surface.alt" />
      <Button label="Open dialog" onPress={() => dialog.set(true)} />
      <Dialog open={dialog} title="Size" message="Pick a size." onClose={() => log.set("closed")}
              actions={[<Action label="OK" role="primary" onPress={() => dialog.set(false)} />]}>
        <Slider label="Size" value={size} min={0} max={100} step={1} />
        <TextField label="Name" value={name} />
      </Dialog>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Drag", component: Home } } });
