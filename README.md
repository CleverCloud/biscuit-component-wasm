# Biscuit playground

This is an example application for [Biscuit tokens](https://github.com/clevercloud/biscuit),
where you can manipulate tokens and their verification in your browser.

Generate the npm package:

```
wasm-pack build --target web --out-dir web --out-name biscuit
cd web
npm pack
// edit package.json to add "snippets" to the "files" array
```

