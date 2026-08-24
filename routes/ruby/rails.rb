class UsersController
  def show
  end

  def create
  end
end

module Admin
  class DashboardController
    def index
    end
  end
end

Rails.application.routes.draw do
  get '/users/:id', to: 'users#show'
  post '/users' => 'users#create'

  namespace :admin do
    get '/dashboard', to: 'dashboard#index'
  end
end
