def process(request):
    users = User.objects.all()
    for user in users:
        send(user)
