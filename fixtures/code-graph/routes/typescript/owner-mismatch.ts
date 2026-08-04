import express from "express";

const app = express();

app.get("/owner-mismatch", MissingController.show);

class ExistingController {
  show() {
    return "existing";
  }
}
