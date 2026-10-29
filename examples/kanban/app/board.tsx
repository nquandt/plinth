// The board screen: the summary, the filter bar, and the four columns.
//
// The screen uses the whole window width (`layout="full"`). Wide and
// regular windows show the columns side by side: they share the width, and
// each has a floor (`minWidth`); a Scroll scrolls sideways if they do not
// fit. A compact window shows one column, chosen with the switch above it:
// the other columns are `hidden` (UI API 1.14) there, and the switch is
// `hidden` on the wider classes. A hidden element has no space, no tab
// stop and no accessibility node.
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

/** The least width of a column, in spacing units. The columns share the width of the window above it. */
const COL_MIN = 52;
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
      pressed={on()}
      hover={{ border: "accent" }}
      focus={{ border: "accent" }}
      onPress={() => togglePriorityFilter(priority)}
    >
      <Span size="sm" weight={on() ? "semibold" : "regular"} fg={on() ? "accent" : "text"}>{priority}</Span>
    </Pressable>
  );
}

function filterBar() {
  return (
    <Box gap={2} padding={3} radius="md" bg="surface" border="border" label="Filters">
      <TextField label="Search" placeholder="Search title, description, labels" value={search} />
      <Picker label="Assigned to" options={["Anyone", "Ada", "Grace", "Alan"]} value={assigneeFilter} />
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

/** One button of the switch: the column name, and its count below it (the name never wraps). */
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
      selected={chosen() === c}
      border={chosen() === c ? "accent" : "border"}
      focus={{ border: "text" }}
      onPress={() => chosen.set(c)}
    >
      <Span size="sm" weight="semibold" lines={1} fg={chosen() === c ? "accent" : "text"}>{name}</Span>
      <Span size="xs" lines={1} fg={chosen() === c ? "accent" : "text.muted"}>{`${counts()[c]}`}</Span>
    </Pressable>
  );
}

/** The switch of the compact window. A wider window hides it (UI API 1.14 `hidden`). */
function columnSwitch() {
  return (
    <Box label="Choose a column" direction="row" gap={1} regular={{ hidden: true }} wide={{ hidden: true }}>
      {switchButton(0)}
      {switchButton(1)}
      {switchButton(2)}
      {switchButton(3)}
    </Box>
  );
}

function column(c: number) {
  const name = COLUMN_NAMES[c];
  const id = COLUMNS[c];
  return (
    <Box
      label={`${name} column`}
      grow={1}
      minWidth={COL_MIN}
      hidden={chosen() !== c}
      regular={{ hidden: false }}
      wide={{ hidden: false }}
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
    </Box>
  );
}

export default function Board() {
  return (
    <Screen
      title="Board"
      layout="full"
      actions={[
        <Action label="New issue" icon="plus" role="primary" onPress={() => startCreate("backlog")} />,
        <Action label="Add 100 demo issues" onPress={() => { addDemoIssues(100); }} />,
        <Action label="Add 500 demo issues" onPress={() => { addDemoIssues(500); }} />,
      ]}
    >
      <Section title="Issues">
        {filterBar()}
        {columnSwitch()}
        <Scroll label="Board" direction="row" gap={3} paddingY={1}>
          {column(0)}
          {column(1)}
          {column(2)}
          {column(3)}
        </Scroll>
        <Span size="xs" fg="text.muted">
          On a card: Enter opens it, E edits, Delete removes, U and D move it up and down, the arrow keys move it to another column. The menu key opens the card menu.
        </Span>
      </Section>
      <Section title="Summary">
        <Box direction="row" gap={4} align="center" compact={{ direction: "column", align: "stretch" }}>
          <Box grow={1}>
            <Chart label="Issues per column" kind="bar" data={chartData()} />
          </Box>
          <Box grow={1} gap={2}>
            <Text>{summary()}</Text>
            <Progress label="Done" value={doneShare()} />
            {status() !== "" ? <Text tone={status().startsWith("Loading") ? "muted" : "danger"}>{status()}</Text> : null}
          </Box>
        </Box>
      </Section>
      {editor()}
    </Screen>
  );
}
