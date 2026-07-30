class UsersController
  def show
  end

  def create
  end
end

class DashboardController
  def index
  end
end

Rails.application.routes.draw do
  get '/users/:id', to: 'users#show'
  post '/users' => 'users#create'

  namespace :admin do
    get '/dashboard', to: 'dashboard#index'
  end
end
