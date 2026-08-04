from fastapi import APIRouter, Depends, FastAPI

app = FastAPI()
router = APIRouter(prefix="/v1")
app.include_router(router, prefix="/api")


def authenticate():
    return True


@app.get("/health")
def health():
    return {"ok": True}


@router.post("/users", dependencies=[Depends(authenticate)])
def create_user():
    return {"created": True}
