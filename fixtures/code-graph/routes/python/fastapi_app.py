from fastapi import APIRouter, Depends, FastAPI

app = FastAPI()
router = APIRouter(prefix="/v1")


def authenticate():
    return True


@app.get("/health")
def health():
    return {"ok": True}


@router.post("/users", dependencies=[Depends(authenticate)])
def create_user():
    return {"created": True}
