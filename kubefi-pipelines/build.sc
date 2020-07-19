import mill._, scalalib._

object core extends ScalaModule {
  def scalaVersion = "2.13.3"

  // use `::` for scala deps, `:` for java deps
  def ivyDeps =
    Agg(
      ivy"com.goyeau::kubernetes-client:0.4.0"              
    )

  object test extends Tests {
    def ivyDeps =
      Agg(
        ivy"org.scalactic::scalactic:3.1.1",
        ivy"org.scalatest::scalatest:3.1.1"
      )
    def testFrameworks = Seq("org.scalatest.tools.Framework")
  }
}
