from fastapi import FastAPI

app = FastAPI()
PREFIX = "/computed"


def route(path):
    return lambda function: function


@route("/not-a-framework-route")
def ordinary_decorator():
    return None


URL = "/also-not-a-route"


def computed(prefix, app, handler):
    app.route(prefix + "/dynamic")(handler)


@app.get(PREFIX + "/decorator")
def computed_decorator():
    return None
