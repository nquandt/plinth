import {
  signal,
  navigate,
  Screen,
  Section,
  Action,
  Sheet,
  TextField,
  NumberField,
  Picker,
  DatePicker,
  Button,
  Text,
  List,
  Row,
  Empty,
} from "plinth:ui";
import {
  filterCategory,
  search,
  filteredTransactions,
  transactions,
  income,
  spending,
  balance,
  selectedTransaction,
  addTransaction,
  updateTransaction,
  deleteTransaction,
  selectTransaction,
} from "./model";
import { formatDate, now } from "plinth:time";
import type { Category } from "./types";

function today(): string {
  return formatDate(now(), "YYYY-MM-DD", false);
}

function amountTrailing(amount: number): string {
  return amount >= 0 ? `+${amount.toFixed(2)}` : amount.toFixed(2);
}

function rowSubtitle(date: string, note: string): string {
  return note.trim() === "" ? date : `${date} · ${note}`;
}

export default function Transactions() {
  const addOpen = signal(false);
  const editOpen = signal(false);

  const draftDate = signal(today());
  const draftAmount = signal(0);
  const draftCategory = signal<string>("Groceries");
  const draftNote = signal("");

  const editDate = signal("");
  const editAmount = signal(0);
  const editCategory = signal<string>("Groceries");
  const editNote = signal("");

  const openAdd = () => {
    draftDate.set(today());
    draftAmount.set(0);
    draftCategory.set("Groceries");
    draftNote.set("");
    addOpen.set(true);
  };

  const submitAdd = () => {
    if (addTransaction(draftDate(), draftAmount(), draftCategory() as Category, draftNote())) {
      addOpen.set(false);
    }
  };

  const openEdit = (id: number) => {
    selectTransaction(id);
    const t = selectedTransaction();
    if (t === null) {
      return;
    }
    editDate.set(t.date);
    editAmount.set(t.amount);
    editCategory.set(t.category);
    editNote.set(t.note);
    editOpen.set(true);
  };

  const submitEdit = () => {
    const t = selectedTransaction();
    if (t === null) {
      return;
    }
    updateTransaction(t.id, editDate(), editAmount(), editCategory() as Category, editNote());
    editOpen.set(false);
  };

  const removeSelected = () => {
    const t = selectedTransaction();
    if (t === null) {
      return;
    }
    deleteTransaction(t.id);
    editOpen.set(false);
  };

  return (
    <Screen title="Budget" actions={[<Action label="Add" icon="plus" onPress={openAdd} />]}>
      <Section title="Summary">
        <Text>{`Income: ${income().toFixed(2)}`}</Text>
        <Text>{`Spending: ${spending().toFixed(2)}`}</Text>
        <Text tone={balance() >= 0 ? "success" : "danger"}>{`Balance: ${balance().toFixed(2)}`}</Text>
        <Button label="View statistics" onPress={() => navigate.push("stats")} />
      </Section>
      <Section title="Filter">
        <Picker label="Category" value={filterCategory} options={["All", "Groceries", "Rent", "Transport", "Fun", "Salary", "Other"]} />
        <TextField label="Search" placeholder="Search notes or category" value={search} />
      </Section>
      <Section title="Transactions">
        <Text tone="muted">{`${filteredTransactions().length} of ${transactions().length} transactions`}</Text>
        <List
          items={filteredTransactions()}
          key={(t) => t.id}
          row={(t) => (
            <Row
              title={t.category}
              subtitle={rowSubtitle(t.date, t.note)}
              trailing={amountTrailing(t.amount)}
              onPress={() => openEdit(t.id)}
            />
          )}
          empty={<Empty title="No transactions" message="Add one with the + button above." />}
        />
      </Section>

      <Sheet open={addOpen} title="Add transaction" onClose={() => addOpen.set(false)}>
        <DatePicker label="Date" value={draftDate} />
        <NumberField label="Amount" value={draftAmount} step={0.01} />
        <Picker label="Category" value={draftCategory} options={["Groceries", "Rent", "Transport", "Fun", "Salary", "Other"]} />
        <TextField label="Note" placeholder="Optional" value={draftNote} />
        <Button label="Add" role="primary" onPress={submitAdd} disabled={draftDate().trim() === "" || draftAmount() === 0} />
      </Sheet>

      <Sheet open={editOpen} title="Edit transaction" onClose={() => editOpen.set(false)}>
        <DatePicker label="Date" value={editDate} />
        <NumberField label="Amount" value={editAmount} step={0.01} />
        <Picker label="Category" value={editCategory} options={["Groceries", "Rent", "Transport", "Fun", "Salary", "Other"]} />
        <TextField label="Note" value={editNote} />
        <Button label="Save" role="primary" onPress={submitEdit} />
        <Button label="Delete" role="destructive" onPress={removeSelected} />
      </Sheet>
    </Screen>
  );
}
