// 7GUIs task 3, Flight booker
// (https://eugenkiss.github.io/7guis/tasks#flight).
//
// A one-way flight needs a start date. A return flight also needs a
// return date that is not before the start date. A date that is not valid
// shows an error. "Book" is enabled only when the input is valid.
import { app, signal, computed, Screen, Section, Picker, TextField, Button, Dialog, Action } from "plinth:ui";
import { now, formatDate } from "plinth:time";
import { parseDay } from "./dates";

const ONE_WAY = "one-way flight";
const RETURN = "return flight";

function FlightBooker() {
  const today = formatDate(now(), "DD.MM.YYYY");
  const kind = signal(ONE_WAY);
  const start = signal(today);
  const back = signal(today);
  const booked = signal(false);
  const message = signal("");

  const startDay = computed(() => parseDay(start()));
  const backDay = computed(() => parseDay(back()));
  const isReturn = computed(() => kind() === RETURN);
  const canBook = computed(() => {
    const s = startDay();
    if (s === null) {
      return false;
    }
    if (!isReturn()) {
      return true;
    }
    const b = backDay();
    return b !== null && b >= s;
  });
  const backError = computed(() => {
    const b = backDay();
    if (b === null) {
      return "Use the form DD.MM.YYYY";
    }
    const s = startDay();
    return s !== null && b < s ? "The return date is before the start date" : "";
  });

  const book = () => {
    if (!canBook()) {
      return;
    }
    message.set(
      isReturn()
        ? `You booked a return flight on ${start().trim()}, back on ${back().trim()}.`
        : `You booked a one-way flight on ${start().trim()}.`,
    );
    booked.set(true);
  };

  return (
    <Screen title="Flight Booker">
      <Section>
        <Picker label="Flight" value={kind} options={[ONE_WAY, RETURN]} />
        <TextField
          label="Start date"
          placeholder="DD.MM.YYYY"
          value={start}
          error={startDay() === null ? "Use the form DD.MM.YYYY" : ""}
        />
        {isReturn() && <TextField label="Return date" placeholder="DD.MM.YYYY" value={back} error={backError()} />}
        <Button label="Book" role="primary" disabled={!canBook()} onPress={book} />
      </Section>
      <Dialog
        open={booked}
        title="Flight booked"
        message={message()}
        actions={[<Action label="OK" role="primary" onPress={() => booked.set(false)} />]}
      />
    </Screen>
  );
}

export default app({
  accent: "blue",
  screens: {
    booker: { title: "Flight Booker", icon: "calendar", component: FlightBooker },
  },
});
