# {{name}}

A [Plinth](https://github.com/nquandt/plinth) app.

```sh
npm run dev     # run the app; it reloads when you save
npm run check   # type-check
npm run build   # make dist/{{slug}}.plnt
```

Without npm, run the same commands with the `plinth` binary: `plinth dev`, `plinth check`, `plinth build`.

The app code is in `app/`. `app/main.tsx` exports the app. The app metadata is in `plinth.toml`.
