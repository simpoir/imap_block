imap_blocks is an i3 block command for inspecting imap state.

It takes a mutt-formatted config file in paramameter, looking for something
like so:

```
set imap_user = 'my_user'
set imap_pass = 'my_pass'
set folder    = imaps://imap.gmail.com/
```

It'll try to idle. It'll try to poll. It'll retry with some backoff.

And that's pretty much it.
