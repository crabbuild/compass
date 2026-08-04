import { Hono } from "hono";

const app = new Hono();

function health() {
  return app.text("ok");
}

app.get("/health", health);
