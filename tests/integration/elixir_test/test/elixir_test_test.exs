defmodule ElixirTestTest do
  use ExUnit.Case, async: false
  doctest ElixirTest
  alias ElixirTest.Encrypted
  alias ElixirTest.Repo
  import Ecto.Query

  setup do
    :ok = Ecto.Adapters.SQL.Sandbox.checkout(Repo)
  end

  test "db connection test" do
    result = Ecto.Adapters.SQL.query!(Repo, "SELECT 1 as one")

    assert result.rows == [[1]]
  end

  test "plaintext save and load" do
    {:ok, result} =
      %Encrypted{plaintext: "plaintext content", plaintext_date: ~D[2025-06-02]}
      |> Repo.insert()

    fetched = Encrypted |> Repo.get(result.id)

    assert fetched.plaintext == "plaintext content"
    assert fetched.plaintext_date == ~D[2025-06-02]
  end

  test "encrypted text save and load" do
    {:ok, result} =
      %Encrypted{encrypted_text: "encrypted text content"}
      |> Repo.insert()

    fetched = Encrypted |> Repo.get(result.id)

    assert fetched.encrypted_text == "encrypted text content"
  end

  test "encrypted fields save and load" do
    {:ok, result} =
      %Encrypted{
        encrypted_bool: false,
        encrypted_int2: 2,
        encrypted_int4: 4,
        encrypted_int8: 8,
        encrypted_float8: 3.1415,
        encrypted_date: ~D[2025-06-01],
        encrypted_jsonb: %{top: %{array: [1, 2, 3]}}
      }
      |> Repo.insert()


    fetched = Encrypted |> Repo.get(result.id)

    assert !fetched.encrypted_bool
    assert fetched.encrypted_int2 == 2
    assert fetched.encrypted_int4 == 4
    assert fetched.encrypted_int8 == 8
    assert fetched.encrypted_float8 == 3.1415
    assert fetched.encrypted_date == ~D[2025-06-01]
    assert fetched.encrypted_jsonb == %{"top" => %{"array" => [1, 2, 3]}}
  end

  test "find by exact text" do
    {2, _} =
      Encrypted
      |> Repo.insert_all([
        %{encrypted_text: "encrypted text content"},
        %{encrypted_text: "some other encrypted text"}
      ])

    q =
      from e in Encrypted,
        where: e.encrypted_text == "encrypted text content",
        select: [e.encrypted_text]

    fetched = Repo.all(q)

    assert Enum.at(fetched, 0) == ["encrypted text content"]
  end

  test "find by text match" do
    {2, _} =
      Encrypted
      |> Repo.insert_all([
        %{encrypted_text: "encrypted text content"},
        %{encrypted_text: "some other encrypted text"}
      ])

    q =
      from e in Encrypted,
        where: like(e.encrypted_text, "text cont"),
        select: [e.encrypted_text]

    fetched = Repo.all(q)

    assert Enum.at(fetched, 0) == ["encrypted text content"]
  end

  test "find by float value - currently not supported" do
    {2, _} =
      Encrypted
      |> Repo.insert_all([
        %{encrypted_float8: 0.0},
        %{encrypted_float8: 7.5}
      ])

    # Ecto appends explicit cast to `7.5`, making it `7.5::float` and causes
    # the "operator does not exist" error
    q =
      from e in Encrypted,
        where: e.encrypted_float8 == 7.5,
        select: [e.id, e.encrypted_float8]

    assert_raise(Postgrex.Error, fn -> Repo.all(q) end)
  end

  test "find by float value" do
    {2, _} =
      Encrypted
      |> Repo.insert_all([
        %{encrypted_float8: 0.0},
        %{encrypted_float8: 7.5}
      ])

    q =
      from e in Encrypted,
        where: fragment("? = 7.5", e.encrypted_float8),
        select: [e.encrypted_float8]

    fetched = Repo.all(q)

    assert Enum.at(fetched, 0) == [7.5]
  end

  test "find by float value gt" do
    {2, _} =
      Encrypted
      |> Repo.insert_all([
        %{encrypted_float8: 0.0},
        %{encrypted_float8: 7.5}
      ])

    q =
      from e in Encrypted,
        where: fragment("? > 3.0", e.encrypted_float8),
        select: [e.encrypted_float8]

    fetched = Repo.all(q)

    assert Enum.at(fetched, 0) == [7.5]
  end

  test "order by integer" do
    {3, _} =
      Encrypted
      |> Repo.insert_all([
        %{encrypted_int2: 7},
        %{encrypted_int2: 9},
        %{encrypted_int2: 0}
      ])

    q =
      from e in Encrypted,
        order_by: e.encrypted_int2,
        select: [e.encrypted_int2]

    fetched = Repo.all(q) |> List.flatten()

    assert fetched == [0, 7, 9]
  end

  test "find by text and float" do
    {3, _} =
      Encrypted
      |> Repo.insert_all([
        %{encrypted_text: "encrypted text content", encrypted_float8: 1.0},
        %{encrypted_text: "encrypted text content", encrypted_float8: 3.0},
        %{encrypted_text: "some other encrypted text", encrypted_float8: 5.0}
      ])

    q =
      from e in Encrypted,
        where: like(e.encrypted_text, "text cont"),
        where: fragment("? > 2.0", e.encrypted_float8),
        select: [e.encrypted_text, e.encrypted_float8]

    fetched = Repo.all(q)

    assert Enum.at(fetched, 0) == ["encrypted text content", 3.0]
  end
end
