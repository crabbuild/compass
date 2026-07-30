from flask import Blueprint, Flask

app = Flask(__name__)
api = Blueprint("api", __name__, url_prefix="/api")


@app.route("/health")
def health():
    return {"ok": True}


@api.route("/users/<user_id>", methods=["GET", "PATCH"])
def user_detail(user_id):
    return {"id": user_id}
