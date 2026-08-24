from django.urls import path


def qualification_health(request):
    return True


urlpatterns = [
    path("qualification-health/", qualification_health),
]
