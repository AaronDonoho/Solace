# Default board

The Lustre client uses only the Spec 15 HTTP API. `solace serve` embeds the
built HTML and JavaScript, so an installed binary needs no asset directory.

Build after changing the client:

```
gleam build --target javascript
bun build src/main.mjs --outfile priv/app.js --minify --target browser
```

Check it without Solace or an agent:

```
gleam test --target javascript
bun test test/api.test.mjs
```
