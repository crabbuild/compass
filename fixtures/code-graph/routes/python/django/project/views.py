def user_detail(request, user_id):
    return user_id


def health(request):
    return True


class AccountView:
    @classmethod
    def as_view(cls):
        return cls
