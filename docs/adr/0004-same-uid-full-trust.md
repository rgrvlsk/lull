# Every same-uid socket client is fully trusted; no caller roles

The daemon socket (`0600`) accepts any process of the current user, including coding
agents, and gives every client the same methods: an agent may pin a server, change config
or start a server with `LULL_KEEP=1`. We considered identifying the peer with
`LOCAL_PEERPID` and applying the agent-marker rule (ADR-0001) to deny agents `pin` and
`config.set`, but rejected it: anything running as the user can already `kill`, edit
`config.toml` or unload the LaunchAgent, so roles would add complexity without a real
boundary. The trust boundary is the uid, and the menu-bar app keeps the user informed by
showing every live server, every pin and every recent stop, so an agent quietly
neutralising lull is visible rather than prevented.
