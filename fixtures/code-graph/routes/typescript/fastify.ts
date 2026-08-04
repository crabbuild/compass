import fastify from "fastify";

const app = fastify();

function health() {
  return "ok";
}

app.get("/health", health);
