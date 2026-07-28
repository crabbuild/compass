const app = createUnrelatedClient();
const path = "/looks-like-a-route";

app.get(path, handler);
client.post("/also-not-a-route", handler);

const arbitrary = {
  path: "/not-a-router-config",
  component: LooksLikeAComponent,
};
