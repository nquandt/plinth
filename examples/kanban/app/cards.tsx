// The pieces of a card: the type badge, the priority, the avatar, the label
// pills, and the card itself. Level 2 primitives with token colors only.
import { Box, Span, Pressable } from "plinth:ui";
import { dropOnCard, openDetail, openMenu, cardKey } from "./model";
import { keyOf, labelsOf, initials } from "./types";
import type { Issue } from "./types";

/** The height of a card, in spacing units. A fixed height lets the host build only the cards in view. */
export const CARD_H = 33;

/** The token color of a priority. Danger, accent, success and muted text. */
export function priorityColor(priority: string): "danger" | "accent" | "success" | "text.muted" {
  if (priority === "Urgent") {
    return "danger";
  }
  if (priority === "High") {
    return "accent";
  }
  if (priority === "Medium") {
    return "success";
  }
  return "text.muted";
}

/** The mark of a priority. A word or a symbol goes with the color (no meaning by color only). */
export function priorityMark(priority: string): string {
  if (priority === "Urgent") {
    return "!! Urgent";
  }
  if (priority === "High") {
    return "^ High";
  }
  if (priority === "Medium") {
    return "= Medium";
  }
  return "v Low";
}

export function kindColor(kind: string): "danger" | "accent" | "text.muted" {
  return kind === "Bug" ? "danger" : kind === "Story" ? "accent" : "text.muted";
}

export function typeBadge(kind: string) {
  return (
    <Box paddingX={2} radius="sm" border={kindColor(kind)} justify="center">
      <Span size="xs" weight="medium" fg={kindColor(kind)}>{kind}</Span>
    </Box>
  );
}

export function avatar(name: string) {
  return (
    <Box label={name === "" ? "Unassigned" : `Assigned to ${name}`} width={7} height={7} radius="full" align="center" justify="center"
         bg={name === "" ? "none" : "accent"} border={name === "" ? "border" : "none"}>
      <Span size="xs" weight="semibold" fg={name === "" ? "text.muted" : "on.accent"}>{initials(name)}</Span>
    </Box>
  );
}

export function pill(text: string) {
  return (
    <Box paddingX={2} radius="full" bg="surface.alt">
      <Span size="xs" fg="text.muted" lines={1}>{text}</Span>
    </Box>
  );
}

/** One card. It is a button (opens the detail), a drag source, a drop target (drop before or after it), and has a context menu and keys. */
export function card(i: Issue) {
  return (
    <Pressable
      label={`${keyOf(i)}: ${i.title}`}
      role="button"
      height={CARD_H}
      padding={3}
      gap={2}
      radius="md"
      bg="surface"
      border="border"
      hover={{ border: "accent" }}
      focus={{ border: "accent" }}
      dragValue={`${i.id}`}
      onDrop={(value, x, y) => dropOnCard(value, i, y, CARD_H)}
      onPress={() => openDetail(i.id)}
      onContextMenu={() => openMenu(i.id)}
      onKeyDown={(key) => cardKey(i, key)}
    >
      <Box direction="row" gap={2} align="center">
        <Span size="xs" mono={true} fg="text.muted" grow={1}>{keyOf(i)}</Span>
        {typeBadge(i.kind)}
      </Box>
      <Box height={11}><Span weight="medium" lines={2}>{i.title}</Span></Box>
      <Box direction="row" gap={1} align="center" justify="between">
        <Span size="xs" weight="medium" fg={priorityColor(i.priority)}>{priorityMark(i.priority)}</Span>
        <Box direction="row" gap={1} align="center">
          {labelsOf(i).slice(0, 1).map((l) => pill(l))}
          {avatar(i.assignee)}
        </Box>
      </Box>
    </Pressable>
  );
}
