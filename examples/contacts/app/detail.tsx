import { signal, navigate, Action, Screen, Section, Sheet, TextField, Button, Text } from "plinth:ui";
import { selectedContact, updateContact, deleteContact, toggleFavorite } from "./model";

// Plain helper functions, not JSX components: called directly from a prop
// or text-child expression, so the compiler wraps the call in an effect
// that reruns when `selected` or `contacts` changes (SPEC.md §7.1). A
// signal read outside such a position is not reactive (SPEC.md §6
// decisions), so the handlers below re-read `selectedContact()` instead of
// closing over a value read at component-call time.
function detailTitle(): string {
  const c = selectedContact();
  if (c === null) {
    return "Contact";
  }
  return c.name;
}

function favoriteLabel(): string {
  const c = selectedContact();
  if (c === null || !c.favorite) {
    return "Not a favorite";
  }
  return "Favorite";
}

function favoriteButtonLabel(): string {
  const c = selectedContact();
  if (c === null || !c.favorite) {
    return "Make favorite";
  }
  return "Unfavorite";
}

export default function ContactDetail() {
  const editOpen = signal(false);
  const editName = signal("");
  const editPhone = signal("");

  const openEdit = () => {
    const c = selectedContact();
    if (c === null) {
      return;
    }
    editName.set(c.name);
    editPhone.set(c.phone);
    editOpen.set(true);
  };

  const saveEdit = () => {
    const c = selectedContact();
    if (c === null) {
      return;
    }
    updateContact(c.id, editName(), editPhone());
    editOpen.set(false);
  };

  const flipFavorite = () => {
    const c = selectedContact();
    if (c === null) {
      return;
    }
    toggleFavorite(c.id);
  };

  const removeContact = () => {
    const c = selectedContact();
    if (c === null) {
      return;
    }
    deleteContact(c.id);
    navigate.back();
  };

  return (
    <Screen
      title={detailTitle()}
      actions={[
        <Action label="Edit" icon="gear" onPress={openEdit} />,
        <Action label="Delete" role="destructive" onPress={removeContact} />,
      ]}
    >
      <Section title="Details">
        <Text>{selectedContact()?.phone ?? ""}</Text>
        <Text tone="muted">{favoriteLabel()}</Text>
        <Button label={favoriteButtonLabel()} onPress={flipFavorite} />
      </Section>
      <Sheet open={editOpen} title="Edit contact" onClose={() => editOpen.set(false)}>
        <TextField label="Name" value={editName} />
        <TextField label="Phone" value={editPhone} />
        <Button label="Save" role="primary" onPress={saveEdit} />
      </Sheet>
    </Screen>
  );
}
