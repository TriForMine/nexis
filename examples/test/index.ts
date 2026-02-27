import { connect } from "@triformine/nexis-sdk";

const client = await connect("ws://localhost:4000", {
  projectId: "7c33d937-409e-4c36-838e-77a2de552133",
  token:
    "eyJwcm9qZWN0X2lkIjoiN2MzM2Q5MzctNDA5ZS00YzM2LTgzOGUtNzdhMmRlNTUyMTMzIiwiaXNzdWVkX2F0IjoiMjAyNi0wMi0yN1QxNDo0ODoxMC4wNzBaIiwiZXhwaXJlc19hdCI6IjIwMjYtMDItMjdUMTU6NDg6MTAuMDcwWiIsImtleV9pZCI6IjUyZWJmZTM2LTYwNGUtNDI4ZC1hMDNiLTI3Zjc2Yzc5ZGE2NiJ9.Ttk_W0XWS6ZKdmsVO5gg-i0GpDFANzuG6ulTj1kxm7g",
});

console.log("hi");

const room = await client.joinOrCreate("counter_plugin_room", {
  roomId: "counter_plugin_room:default",
});

console.log("hi 2");

room.onStateChange((state) => {
  console.log("state", state);
});

room.send("inc", { by: 1 });

console.log("started");
