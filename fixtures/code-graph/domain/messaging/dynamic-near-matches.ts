class EventPattern {
  constructor(value: string) {}
}

class Fake {
  handle() {
    new EventPattern(getDynamicName());
  }
}
