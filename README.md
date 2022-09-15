This repo is a demonstration of how you could build your own "serverless" platform with data persistence in only 200~ lines of simple, safe code.

It combines [deno](https://deno.land/) (rust v8 runtime), [axum](https://github.com/tokio-rs/axum/) (http library), and [rusqlite](https://github.com/rusqlite/rusqlite) (sqlite) to build a Function as a Service (FaaS)-like application with file-system backed storage.

A user submits javascript code with a HTTP POST request, then is able to invoke their function via GET requests.

The js environment the code is executed within has access to 3 operations implemented in rust. `console.log` emits messages via stdout on the server, `set` inserts into a sqlite backed key-value store, and `get` retrieves from the same store.

The last evaluated expression from an invoked js function will be returned in the HTTP GET response body.

### Example



In your first terminal

```
cargo run
```

In your second terminal
```bash
# Submit the contents of fn.js as the function "raccoon"
curl -d @fn.js localhost:8080/fn/raccoon

# Invoke the function by it's name
curl localhost:8080/fn/raccoon

# And a couple more times... just in case
curl localhost:8080/fn/raccoon
curl localhost:8080/fn/raccoon
curl localhost:8080/fn/raccoon
```