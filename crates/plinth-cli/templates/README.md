# {{name}}

A [Plinth](https://github.com/nquandt/plinth) app: a small task list with a
pushed detail screen, split into `app/model.ts` (state and persistence),
`app/tasks.tsx` (the list screen) and `app/detail.tsx` (the pushed detail
screen), wired up in `app/main.tsx`.

```sh
npm run dev     # run the app; it reloads when you save
npm run check   # type-check
npm run build   # make dist/{{slug}}.plnt
```

Without npm, run the same commands with the `plinth` binary: `plinth dev`,
`plinth check`, `plinth build`.

Tasks save to the device with the `store.kv` capability (declared in
`plinth.toml`), so they are still there next time the app runs. The app
metadata, including capabilities, is in `plinth.toml`.
