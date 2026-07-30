import express from "express";
import { authenticate, audit, showUser } from "./handlers";

const app = express();
const router = express.Router();

app.get("/health", health);
router.get("/users/:userId", authenticate, audit(), showUser);
app.get(dynamicPath, ignoredHandler);
unknown.get("/not-a-route", ignoredHandler);

function health() {
  return "ok";
}
