from starlette.applications import Starlette
from starlette.routing import Mount, Route, Router


def health(request):
    return {"ok": True}


child = Router(routes=[Route("/health", health)])
app = Starlette(routes=[Mount("/api", app=child)])
