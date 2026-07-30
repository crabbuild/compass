class Metrics
  def get(path, to:)
  end
end

get '/not-rails', to: 'metrics#show'
