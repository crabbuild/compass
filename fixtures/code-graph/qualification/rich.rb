module Billing
  module Auditable
    def audit; end
  end

  class Document
    def save; end
  end

  class Invoice < Document
    include Auditable
    prepend Serializable
    extend ClassMethods

    attr_accessor :number

    def initialize(number, *rest, tax: 0, &block)
      @number = number
    end

    def total(amount, tax = 0, *rest, &block)
      save
    end

    def self.build(number)
      new(number)
    end

    alias_method :persist, :save
    define_method(:refresh) { save }
  end
end

class Billing::Invoice
  def reopened; end
end

require_relative "billing/document"
autoload :Serializable, "billing/serializable"

Rails.application.routes.draw do
  namespace :admin do
    get "/invoices", to: "invoices#index"
  end
end
