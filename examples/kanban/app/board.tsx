// The board screen: the summary, the filter bar, and the four columns.
//
// Wide and regular windows show the columns side by side in a Scroll that
// scrolls sideways. A compact window shows one column, chosen with the
// switch above it. The app cannot read the width class, so each column
// carries both looks: its `maxHeight` is 0 when it is not the chosen one,
// and the `regular` and `wide` partial styles undo that.
import {
  signal,
  Screen,
  Section,
  Box,
  Span,
  Pressable,
  Scroll,
  Chart,
  Progress,
  Text,
  TextField,
  Picker,
  Action,
} from "plinth:ui";
import {
  status,
  search,
  assigneeFilter,
  priorityFilter,
  togglePriorityFilter,
  clearFilters,
  filtering,
  board,
  counts,
  chartData,
  summary,
  doneShare,
  shownCount,
  startCreate,
  addDemoIssues,
  dropOnColumn,
} from "./model";
import { COLUMNS, COLUMN_NAMES } from "./types";
import { card } from "./cards";
import { editor } from "./editor";

/** The width of a column, in spacing units. */
const COL_W = 72;
/** The most height of the cards of a column, in spacing units. */
const COL_H = 150;

/** The column that the compact window shows. */
const chosen = signal(0);

function priorityChip(priority: string) {
  const on = () => priorityFilter().includes(priority);
  return (
    <Pressable
      label={`Show ${priority} priority`}
      role="button"
      paddingX={3}
      paddingY={1}
      radius="full"
      border={on() ? "accent" : "border"}
      bg={on() ? "selected" : "surface"}
      hover={{ border: "accent" }}
      focus={{ border: "accent" }}
      onPress={() => togglePriorityFilter(priority)}
    >
      <Span size="sm" weight={on() ? "semibold" : "regular"} fg={on() ? "accent" : "text"}>{on() ? `${priority} (on)` : priority}</Span>
    </Pressable>
  );
}

function filterBar() {
  return (
    <Box gap={2} padding={3} radius="md" bg="surface" border="border" label="Filters">
      <Box direction="row" gap={3} wrap={true} align="end" compact={{ direction: "column", align: "stretch" }}>
        <Box grow={1}>
          <TextField label="Search" placeholder="Search title, description, labels" value={search} />
        </Box>
        <Picker label="Assigned to" options={["Anyone", "Ada", "Grace", "Alan"]} value={assigneeFilter} />
      </Box>
      <Box direction="row" gap={2} wrap={true} align="center">
        <Span size="sm" fg="text.muted">Show priority</Span>
        {priorityChip("Urgent")}
        {priorityChip("High")}
        {priorityChip("Medium")}
        {priorityChip("Low")}
        <Box grow={1} />
        {filtering() ? (
          <Pressable label="Clear filters" role="button" paddingX={2} paddingY={1} radius="md" focus={{ border: "accent" }} onPress={clearFilters}>
            <Span size="sm" fg="accent">Clear filters</Span>
          </Pressable>
        ) : null}
      </Box>
    </Box>
  );
}

/** One button of the switch: the column name and its count. */
function switchButton(c: number) {
  const name = COLUMN_NAMES[c];
  return (
    <Pressable
      label={`Show ${name}`}
      role="button"
      grow={1}
      paddingY={2}
      radius="md"
      align="center"
      bg={chosen() === c ? "accent" : "surface"}
      border={chosen() === c ? "accent" : "border"}
      focus={{ border: "text" }}
      onPress={() => chosen.set(c)}
    >
      <Span size="sm" weight="semibold" fg={chosen() === c ? "on.accent" : "text"}>{`${name} ${counts()[c]}`}</Span>
    </Pressable>
  );
}

/** The switch of the compact window. It has no height on a wider window. */
function columnSwitch() {
  return (
    <Scroll label="Choose a column" direction="row" gap={1} maxHeight={0} compact={{ maxHeight: 100 }}>
      {switchButton(0)}
      {switchButton(1)}
      {switchButton(2)}
      {switchButton(3)}
    </Scroll>
  );
}

function column(c: number) {
  const name = COLUMN_NAMES[c];
  const id = COLUMNS[c];
  return (
    <Scroll
      label={`${name} column`}
      width={COL_W}
      maxHeight={chosen() === c ? 400 : 0}
      compact={{ width: "full" }}
      regular={{ maxHeight: 400 }}
      wide={{ maxHeight: 400 }}
    >
      <Box padding={2} gap={2} radius="lg" bg="surface.alt" width="full" onDrop={(value) => dropOnColumn(value, id)}>
        <Box direction="row" align="center" gap={2} paddingX={1}>
          <Span weight="semibold" grow={1}>{name}</Span>
          <Span size="sm" fg="text.muted">{filtering() ? `${shownCount(c)} of ${counts()[c]}` : `${counts()[c]}`}</Span>
          <Pressable label={`Add issue to ${name}`} role="button" width={7} height={7} radius="full" align="center" justify="center"
                     hover={{ bg: "hover" }} focus={{ border: "accent" }} onPress={() => startCreate(id)}>
            <Span weight="bold" fg="accent">+</Span>
          </Pressable>
        </Box>
        <Scroll label={`${name} issues`} gap={2} maxHeight={COL_H} onDrop={(value) => dropOnColumn(value, id)}>
          {board()[c].map((i) => card(i))}
          {board()[c].length === 0 ? (
            <Box padding={3} align="center" height={12} justify="center">
              <Span size="sm" fg="text.muted">{filtering() ? "No match" : "Drop an issue here"}</Span>
            </Box>
          ) : null}
        </Scroll>
      </Box>
    </Scroll>
  );
}

export default function Board() {
  return (
    <Screen
      title="Board"
      actions={[
        <Action label="New issue" icon="plus" role="primary" onPress={() => startCreate("backlog")} />,
        <Action label="Add 100 demo issues" icon="more" onPress={() => addDemoIssues(100)} />,
        <Action label="Add 500 demo issues" icon="more" onPress={() => addDemoIssues(500)} />,
      ]}
    >
      <Section title="Summary">
        <Box direction="row" gap={4} align="center" compact={{ direction: "column", align: "stretch" }}>
          <Box grow={1}>
            <Chart label="Issues per column" kind="bar" data={chartData()} />
          </Box>
          <Box grow={1} gap={2}>
            <Text>{summary()}</Text>
            <Progress label="Done" value={doneShare()} />
            {status() !== "" ? <Text tone="danger">{status()}</Text> : null}
          </Box>
        </Box>
      </Section>
      <Section title="Issues">
        {filterBar()}
        {columnSwitch()}
        <Scroll label="Board" direction="row" gap={3} paddingY={1} compact={{ direction: "column" }}>
          {column(0)}
          {column(1)}
          {column(2)}
          {column(3)}
        </Scroll>
        <Span size="xs" fg="text.muted">
          On a card: Enter opens it, E edits, Delete removes, U and D move it up and down, the arrow keys move it to another column. The menu key opens the card menu.
        </Span>
      </Section>
      {editor()}
    </Screen>
  );
}
