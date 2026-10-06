// Fetches a random quote from a public API (SPEC.md §8.5, §11) and shows
// it. Declares `net:api.quotable.io` in plinth.toml: a request is allowed
// only to a host a declared `net:<host>` capability names.
import { app, Screen, Text, Button, signal } from "plinth:ui";
import { fetch } from "plinth:net";
import { JSON } from "plinth:core";

interface Quote {
  content: string;
  author: string;
}

const quote = signal("Press “New quote” to fetch one.");
const loading = signal(false);

function newQuote() {
  loading.set(true);
  fetch("https://api.quotable.io/random", null, (r) => {
    loading.set(false);
    if (!r.ok) {
      quote.set("Could not fetch a quote: " + (r.error ?? ("HTTP " + r.status)));
      return;
    }
    const parsed = JSON.parse<Quote>(r.text);
    if (parsed === null) {
      quote.set("Could not read the response.");
      return;
    }
    quote.set(parsed.content + " — " + parsed.author);
  });
}

function Home() {
  return (
    <Screen title="Quotes">
      <Text>{quote()}</Text>
      <Button label={loading() ? "Loading…" : "New quote"} onPress={newQuote} />
    </Screen>
  );
}

export default app({ screens: { home: { title: "Quotes", component: Home } } });
