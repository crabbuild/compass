from django.urls import include, path, re_path

from . import views

urlpatterns = [
    path("users/<int:user_id>/", views.user_detail, name="user-detail"),
    re_path(r"^health/$", views.health),
    path("admin/", include("project.admin.urls")),
    path("class/", views.AccountView.as_view()),
]
