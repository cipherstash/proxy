defmodule ElixirTestTest do
  use ExUnit.Case
  doctest ElixirTest

  test "db connection test" do
    result = Ecto.Adapters.SQL.query!(ElixirTest.Repo, "SELECT 1 as one")

    assert result.rows == [[1]]
  end
end
