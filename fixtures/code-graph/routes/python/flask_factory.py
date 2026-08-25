from flask import Blueprint, Flask


root = Blueprint("root", __name__, url_prefix="/root")
nested = Blueprint("nested", __name__, url_prefix="/nested")


@nested.before_request
def authorize():
    return None


@nested.get("/items")
def items():
    return None


root.register_blueprint(nested, url_prefix="/v2")


def create_app():
    app = Flask(__name__)
    app.register_blueprint(root, url_prefix="/api")
    return app
