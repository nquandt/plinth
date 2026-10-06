import { signal, navigate, Screen, Section, Tabs, TextField, Button, Text, List, Row, Empty } from "plinth:ui";
import { tab, visibleContacts, contacts, addContact, selectContact } from "./model";

export default function ContactList() {
  const draftName = signal("");
  const draftPhone = signal("");

  const submit = () => {
    if (addContact(draftName(), draftPhone())) {
      draftName.set("");
      draftPhone.set("");
    }
  };

  const openContact = (id: number) => {
    selectContact(id);
    navigate.push("detail");
  };

  return (
    <Screen title="Contacts">
      <Tabs items={["all", "favorites"]} value={tab} />
      <Section title="All contacts">
        <Text tone="muted">{`${visibleContacts().length} of ${contacts().length} contacts`}</Text>
        <List
          items={visibleContacts()}
          key={(c) => c.id}
          row={(c) => <Row title={c.name} subtitle={c.phone} onPress={() => openContact(c.id)} />}
          empty={<Empty title="No contacts" message="Add one below." />}
        />
      </Section>
      <Section title="Add contact">
        <TextField label="Name" placeholder="Full name" value={draftName} onSubmit={submit} />
        <TextField label="Phone" placeholder="Phone number" value={draftPhone} onSubmit={submit} />
        <Button label="Add contact" role="primary" onPress={submit} disabled={draftName().trim() === ""} />
      </Section>
    </Screen>
  );
}
