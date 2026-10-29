// The overlays of the board: the create/edit dialog, the issue detail sheet,
// and the context menu of a card (the keyboard path of a drag).
import { Dialog, Sheet, Menu, Action, Box, Span, Heading, Text, Button, Pressable, Picker, TextField, TextArea } from "plinth:ui";
import {
  editOpen,
  editingId,
  draftTitle,
  draftDescription,
  draftKind,
  draftPriority,
  draftAssignee,
  draftLabels,
  titleError,
  closeEditor,
  saveDraft,
  detailOpen,
  detail,
  startEdit,
  deleteIssue,
  moveByName,
  menuOpen,
  menuId,
  moveIssue,
  nudgeIssue,
} from "./model";
import { columnName, keyOf, labelsOf } from "./types";
import { priorityColor, priorityMark, typeBadge, avatar, pill } from "./cards";

/** A chip that moves the issue of the sheet to a column. The current column shows as chosen. */
function moveChip(name: string) {
  const here = () => columnName(detail().status) === name;
  return (
    <Pressable label={`Move to ${name}`} role="button" paddingX={3} paddingY={1} radius="full" border={here() ? "accent" : "border"}
               selected={here()} focus={{ border: "accent" }} onPress={() => moveByName(detail().id, name)}>
      <Span size="sm" weight={here() ? "semibold" : "regular"} fg={here() ? "accent" : "text"}>{name}</Span>
    </Pressable>
  );
}

export function editor() {
  return (
    <Box>
      <Dialog
        open={editOpen}
        title={editingId() === 0 ? "New issue" : "Edit issue"}
        onClose={closeEditor}
        actions={[
          <Action label="Save" role="primary" onPress={saveDraft} />,
          <Action label="Cancel" onPress={closeEditor} />,
        ]}
      >
        <TextField label="Title" placeholder="What needs to be done?" value={draftTitle} error={titleError()} onSubmit={saveDraft} />
        <TextArea label="Description" value={draftDescription} />
        <Picker label="Type" options={["Task", "Bug", "Story"]} value={draftKind} />
        <Picker label="Priority" options={["Low", "Medium", "High", "Urgent"]} value={draftPriority} />
        <Picker label="Assignee" options={["Unassigned", "Ada", "Grace", "Alan"]} value={draftAssignee} />
        <TextField label="Labels" placeholder="Separated by commas" value={draftLabels} />
      </Dialog>
      <Sheet open={detailOpen} title={`${keyOf(detail())}`}>
        <Heading level={2}>{detail().title}</Heading>
        <Box direction="row" gap={2} align="center" wrap={true}>
          {typeBadge(detail().kind)}
          <Span size="sm" weight="medium" fg={priorityColor(detail().priority)}>{priorityMark(detail().priority)}</Span>
          <Span size="sm" fg="text.muted">{columnName(detail().status)}</Span>
          {avatar(detail().assignee)}
          <Span size="sm">{detail().assignee === "" ? "Unassigned" : detail().assignee}</Span>
        </Box>
        <Box direction="row" gap={1} wrap={true}>
          {labelsOf(detail()).map((l) => pill(l))}
        </Box>
        <Text>{detail().description === "" ? "No description." : detail().description}</Text>
        <Box direction="row" gap={2} wrap={true} label="Move to a column">
          {moveChip("Backlog")}
          {moveChip("In Progress")}
          {moveChip("Review")}
          {moveChip("Done")}
        </Box>
        <Button label="Edit issue" role="primary" onPress={() => startEdit(detail().id)} />
        <Button label="Delete issue" role="destructive" onPress={() => deleteIssue(detail().id)} />
      </Sheet>
      <Menu
        label="Card menu"
        open={menuOpen}
        actions={[
          <Action label="Move to Backlog" onPress={() => moveByName(menuId(), "Backlog")} />,
          <Action label="Move to In Progress" onPress={() => moveByName(menuId(), "In Progress")} />,
          <Action label="Move to Review" onPress={() => moveByName(menuId(), "Review")} />,
          <Action label="Move to Done" onPress={() => moveByName(menuId(), "Done")} />,
          <Action label="Move up" onPress={() => nudgeIssue(menuId(), -1)} />,
          <Action label="Move down" onPress={() => nudgeIssue(menuId(), 1)} />,
          <Action label="Edit..." onPress={() => startEdit(menuId())} />,
          <Action label="Delete" role="destructive" onPress={() => deleteIssue(menuId())} />,
        ]}
      />
    </Box>
  );
}
